// Polls the internal results service — the same ListArtifacts lookup the
// deploy-pages action runs — until it reports exactly one artifact with the
// given name. deploy-pages makes that lookup without retry, so a deploy that
// starts before the upload's metadata indexes fails with "0 artifacts". The
// public run-artifact list a shell step can poll is a different read path and
// can lag that service, so it is not a safe gate; this waits on the service
// the deploy actually reads.
//
// A node action (unlike a `run:` shell step) receives the short-lived runtime
// token and results-service URL the runner injects, so it can authenticate to
// the internal service. The runner executes the script directly (no npm
// install), so this uses only Node built-ins: global fetch and a manual JWT
// decode.

function int(value, fallback) {
  const n = parseInt(value, 10);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

const artifactName = process.env.INPUT_ARTIFACT_NAME || '';
const timeoutSeconds = int(process.env.INPUT_TIMEOUT_SECONDS, 300);
const pollSeconds = int(process.env.INPUT_POLL_SECONDS, 10);

// The runtime token is a JWT; its payload (base64url) carries the `scp`
// claim, a space-separated list of scopes. The Actions.Results scope is
// `Actions.Results:<runBackendId>:<jobBackendId>` — the two ids the
// ListArtifacts call needs.
function backendIdsFromToken(token) {
  const payload = token.split('.')[1];
  if (!payload) throw new Error('runtime token is not a JWT');
  const { scp } = JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'));
  if (!scp) throw new Error('runtime token has no scp claim');
  for (const scope of scp.split(' ')) {
    const parts = scope.split(':');
    if (parts[0] === 'Actions.Results' && parts.length === 3) {
      return { workflowRunBackendId: parts[1], workflowJobRunBackendId: parts[2] };
    }
  }
  throw new Error('no Actions.Results scope in the runtime token');
}

async function listArtifacts() {
  const token = process.env.ACTIONS_RUNTIME_TOKEN;
  const resultsUrl = process.env.ACTIONS_RESULTS_URL;
  if (!token || !resultsUrl) {
    throw new Error('ACTIONS_RUNTIME_TOKEN / ACTIONS_RESULTS_URL not set');
  }
  const ids = backendIdsFromToken(token);
  const url = new URL(
    '/twirp/github.actions.results.api.v1.ArtifactService/ListArtifacts',
    new URL(resultsUrl).origin,
  ).href;
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      authorization: `bearer ${token}`,
      'content-type': 'application/json',
      'user-agent': '@actions/artifact-2.1.3',
    },
    body: JSON.stringify(ids),
  });
  if (!res.ok) {
    throw new Error(`ListArtifacts HTTP ${res.status}: ${(await res.text()).slice(0, 200)}`);
  }
  const data = await res.json();
  return data.artifacts || [];
}

function fail(message) {
  console.log(`::error::${message}`);
  process.exitCode = 1;
}

async function main() {
  if (!artifactName) {
    fail('no artifact-name input');
    return;
  }
  const deadline = Date.now() + timeoutSeconds * 1000;
  let count = -1;
  for (;;) {
    try {
      const artifacts = await listArtifacts();
      count = artifacts.filter((a) => a.name === artifactName).length;
      console.log(`internal results service reports ${count} "${artifactName}" artifact(s)`);
    } catch (err) {
      console.log(`warning: ListArtifacts failed: ${err.message}`);
    }
    if (count === 1) {
      console.log(`the "${artifactName}" artifact is indexed; the deploy can find it`);
      return;
    }
    if (Date.now() > deadline) {
      fail(`the "${artifactName}" artifact count is ${count} after ${timeoutSeconds}s (want 1)`);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, pollSeconds * 1000));
  }
}

main().catch((err) => fail(`wait-for-artifact failed: ${err.message}`));
