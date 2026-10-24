This is a wip

This plugin is a workaround for the audio issues described in:
https://github.com/bevyengine/bevy/issues/26282

1. https://github.com/RustAudio/rodio/issues/836
    - Resolved in this plugin
2. The app is killed in the background because the audio thread uses too much CPU power.
    - Not resolved, but not tested jet

TODO: The AudioSource append fns in AudioPlayer should take with Handle<AudioSource>.

# Usage
Disable the `audio` feature in bevy and add this `AudioPlugin` instead.

# Example
Run:
```sh
cargo run --example hello_world
```
