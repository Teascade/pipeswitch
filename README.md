# Pipeswitch
Daemon for [PipeWire][pipewire] that automatically links audio inputs and outputs
based on regex for similarly to how [QjackCtl][qjackctl]'s patchbay works on
Jack.

Written to run with `/bin/bash` that watches any new devices, inputs or outputs
logged by `pw-link`, and links them appropriately.

If you're looking for an interactive albeit non-automatic graph GUI for
PipeWire, you might want to check out [Helvum][helvum]

Currently not taking in outside contributions as this is being re-written with
rust with a GUI to manage the configuration file. 

## Features
- Works well as a single service running in the system background
- Is able to update config contents at runtime¹
- Accepts RegEx for matching inputs and outputs

¹: any new links added in the config will only be linked if either of the
      devices is re-introduced for the monitor or the service restarted.

## Config
Configuration is done with a `json` file that is located at
`$XDG_CONFIG_HOME/pipeswitch.json`

The format is following:
```json
{
    "print_links": true, // Prints the links that pipeswitch does
    "debug": false,      // Prints debug information. Currently just all device events
    "links": {
        // key:value pairs of the links you want.
        // key = RegEx of the linked output channel. List with pw-link -o
        // value = RegEx of the linked input channel. List with pw-link -i
        "ExampleFLRegex": "Some Headphones:in_1",
        "ExampleFRRegex": "Some Headphones:in_2"
    }
}
```

If a device has two channels (such as FR/FL), it is usually desirable to create
individual links for both channels.

You can preview what inputs/outputs are currently available with `pw-link -o` and `pw-link -i`

## License
This project is licensed under the [GNU General Public License v3](./LICENSE)

[pipewire]: https://pipewire.org/
[qjackctl]: https://qjackctl.sourceforge.io/
[helvum]: https://gitlab.freedesktop.org/pipewire/helvum
