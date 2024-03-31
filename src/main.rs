use iced::widget::{button, column, text};
use iced::{Element, Sandbox, Settings};
use wasmer::{imports, Instance, Module, Store, Value};

struct Counter {
    value: i32,
    instance: Instance,
    store: Store,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Increment,
    Decrememt,
}

impl Sandbox for Counter {
    type Message = Message;

    fn new() -> Self {
        let module_wat = r#"
            (module
                (func $add
                    (export "add")
                    (param i32)
                    (param i32)
                    (result i32)
                    (i32.add
                        get_local 0
                        get_local 1)))
        "#;
        let mut store = Store::default();
        let module = Module::new(&store, &module_wat).expect("load module failed");
        let import_object = imports! {};
        let instance =
            Instance::new(&mut store, &module, &import_object).expect("instantiate module failed");

        Self {
            value: 0,
            instance,
            store,
        }
    }

    fn title(&self) -> String {
        "waui".to_owned()
    }

    fn update(&mut self, message: Self::Message) {
        let add = self
            .instance
            .exports
            .get_function("add")
            .expect("get add fn failed");
        let amount = match message {
            Message::Increment => 1,
            Message::Decrememt => -1,
        };
        self.value = add
            .call(
                &mut self.store,
                &[Value::I32(self.value), Value::I32(amount)],
            )
            .expect("call add fn failed")[0]
            .unwrap_i32();
    }

    fn view(&self) -> Element<Self::Message> {
        column![
            button("+").on_press(Message::Increment),
            text(self.value),
            button("-").on_press(Message::Decrememt)
        ]
        .into()
    }
}

fn main() -> iced::Result {
    Counter::run(Settings::default())
}
