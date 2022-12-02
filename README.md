# Pipeswitch
Daemon for [PipeWire][pipewire] that automatically links audio inputs and
outputs based on regex for similarly to how [QjackCtl][qjackctl]'s patchbay
works on Jack.

Written entirely in Rust using [official pipewire-bindings][pipewire-bindings],
by implementing listeners similar to those provided by `pw-link`.

If you're looking for an interactive albeit non-automatic graph GUI for
PipeWire, you might want to check out [Helvum][helvum]

## Features
- Works as a single service running in the system background
- Lightning fast
- Is able to hot-reload configuration
    - Optionally destroys links that are no longer configured and
    - Creates new links
- Accepts RegEx for matching inputs and outputs. Able to match client-name,
  node-name and port-names seperately. (Node-name is usually the one you want,
  and the one listed ie. in Helvum)
    - If port-name is not specified, has an option to link ports according to
      their channel (so left-ear matches left-ear)
    - RegEx always expects to match the whole client/node/port-name. (Node-name,
      if in/out is simply a string)

## Config
Configuration is done with a `toml` file that is located at
`$XDG_CONFIG_HOME/pipeswitch.toml`

The format is following:
```toml
# Comments made here should persist through automatic edits.
[general]
# keep links that dont exist in the config anymore
linger_links = false
# inotify listen config and reload when it changes
hotreload_config = true

[log]
level = "trace"

# In and out share the same syntax, both can be expressed as objects or strings.
# Client, Node and Port are technical terms in Pipewire.  
# Always always you're interested in only the Node.
[link.some_default_link]

# Objects have client, node and port -fields, all of which are optional
in = { client = "client_1", node = "node_1" }

# Strings always refer to only the node-name.
out = "Hello there!"

# Optional per-link config  
# if true (default), and ports are not specified in the object-notation, ports
# are connected if they are in the same channel. Left goes into Left, Right into
# Right. Mono only connects to mono even in this special case.
special_empty_ports = true

# A second link for the sake of demonstration
[link.second_link]
in = "Some input"
out = "Hello there!"
```

You can preview what inputs/outputs are currently available with `pw-link -o`
and `pw-link -i` or using Helvum. Note: `pw-link` lists both node-names and port-names.

## License
This project is licensed under the [GNU General Public License v3](./LICENSE)

[pipewire]: https://pipewire.org/
[qjackctl]: https://qjackctl.sourceforge.io/
[helvum]: https://gitlab.freedesktop.org/pipewire/helvum
[pipewire-bindings]: https://crates.io/crates/pipewire
