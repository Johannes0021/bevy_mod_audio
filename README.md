This plugin is a workaround for the audio issues described in:
https://github.com/bevyengine/bevy/issues/26282

1. https://github.com/RustAudio/rodio/issues/836
    - Resolved in this plugin
    - Downside: State tracking and rebuild overhead
2. The app is killed in the background because the audio thread uses too much CPU power.
    - Resolved in this plugin
    - Downside: Tear down full audio backend on suspend and rebuild on resume

# Usage
Disable the `audio` feature in bevy and add this `AudioPlugin` instead.

# Example
Run:
```sh
cargo run --example hello_world
```
