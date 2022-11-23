use std::sync::mpsc::channel;

use pipeswitch_lib::{create_mainloop, tokio};

mod sdl2_signaller;

#[tokio::main]
async fn main() {
    let a = create_mainloop();

    a.unwrap();

    // println!("Hello, world!");
    // let (sender, receiver) = channel();
    // sdl2_signaller::open_window_thread(sender);
    // while let Ok(keycode) = receiver.recv() {
    //     println!("Pressed: {keycode:?}");
    // }
}
