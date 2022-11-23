use std::sync::mpsc::channel;

use pipeswitch_lib::Pipeswitch;

mod sdl2_signaller;

fn main() {
    let ps = Pipeswitch::new(None).unwrap();

    let (sender, receiver) = channel();
    sdl2_signaller::open_window_thread(sender);
    while let Ok(keycode) = receiver.recv() {
        println!("Pressed: {keycode:?}");
        ps.shutdown();
        break;
    }
}
