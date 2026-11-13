//! Proc macros for the `component-ui` crate.
//!
//! They run at the component's compile time only: they generate code and
//! never land in the component's wasm. See `component-ui` for the design.

use proc_macro::TokenStream;

/// Generates the projection from the ergonomic `component-ui` tree to the
/// `Node` of the given `widget` module: a `fn to_node` to pass to
/// `component-ui`'s `lower` and the `changes` diff.
///
/// The argument is the path to the component's `wit-bindgen`-generated
/// `widget` module (the `wit` in `use waui::ui::widget as wit;`).
#[proc_macro]
pub fn to_node(input: TokenStream) -> TokenStream {
    let wit = input.to_string();
    let src = format!(
        "fn to_node(tree: &component_ui::Tree, parent: Option<u32>) -> {wit}::Node {{
            let style = |s: component_ui::Style| {wit}::Style {{
                color: match s.color {{
                    component_ui::Color::Neutral => {wit}::Color::Neutral,
                    component_ui::Color::Accent => {wit}::Color::Accent,
                    component_ui::Color::Positive => {wit}::Color::Positive,
                    component_ui::Color::Negative => {wit}::Color::Negative,
                    component_ui::Color::Warning => {wit}::Color::Warning,
                }},
                size: match s.size {{
                    component_ui::Size::Small => {wit}::Size::Small,
                    component_ui::Size::Medium => {wit}::Size::Medium,
                    component_ui::Size::Large => {wit}::Size::Large,
                }},
            }};
            match tree {{
                component_ui::Tree::Text(t) => {wit}::Node::Text({wit}::Text {{
                    id: t.id,
                    parent,
                    content: t.content.clone(),
                    style: style(t.style),
                }}),
                component_ui::Tree::Button(b) => {wit}::Node::Button({wit}::Button {{
                    id: b.id,
                    parent,
                    label: b.label.clone(),
                    style: style(b.style),
                }}),
                component_ui::Tree::Input(i) => {wit}::Node::Input({wit}::Input {{
                    id: i.id,
                    parent,
                    content: i.content.clone(),
                    style: style(i.style),
                    width: i.width,
                }}),
                component_ui::Tree::Container(c) => {wit}::Node::Container({wit}::Container {{
                    id: c.id,
                    parent,
                    direction: match c.direction {{
                        component_ui::Direction::Row => {wit}::Direction::Row,
                        component_ui::Direction::Column => {wit}::Direction::Column,
                    }},
                    align: match c.align {{
                        component_ui::Align::Start => {wit}::Align::Start,
                        component_ui::Align::Center => {wit}::Align::Center,
                        component_ui::Align::End => {wit}::Align::End,
                    }},
                }}),
            }}
        }}"
    );
    match src.parse::<TokenStream>() {
        Ok(tokens) => tokens,
        // The template is fixed and valid; this fails only on a malformed
        // argument. A panic here is reported at the call site.
        Err(err) => panic!("to_node: the generated code does not parse: {err}"),
    }
}
