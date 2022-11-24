use std::sync::mpsc::channel;

use pipeswitch_lib::Pipeswitch;
use sdl2::keyboard::Keycode;

mod sdl2_signaller;

fn main() {
    let ps = Pipeswitch::new(None).unwrap();

    let (sender, receiver) = channel();
    sdl2_signaller::open_window_thread(sender);
    while let Ok(keycode) = receiver.recv() {
        let state = ps.lock_current_state();
        println!("Doing roundtrip");
        ps.roundtrip().unwrap();
        println!("Current nodes:");
        for (node_id, node) in &state.nodes {
            let node_name = &node.node_name;
            let names: Vec<&String> = state
                .ports_by_node(*node_id)
                .into_iter()
                .map(|p| &p.name)
                .collect();
            println!("Node {node_id} '{node_name}' with ports {names:?}");
        }

        if keycode == Keycode::Escape {
            break;
        }
    }
}
