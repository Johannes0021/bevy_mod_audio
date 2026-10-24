use bevy::prelude::*;
use bevy_mod_audio::{
    prelude::*,
    rodio::{
        ChannelCount, Float, SampleRate, mixer,
        source::{SineWave, Source},
    },
};
use std::time::Duration;

const NOTE_DURATION: Duration = Duration::from_secs(1);
const NOTE_AMPLITUDE: Float = 0.20;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, AudioPlugin::default()))
        .add_systems(Startup, startup)
        .run();
}

fn startup(mut commands: Commands) {
    commands.spawn(AudioPlayer::new().with_append(|position| {
        // Construct a dynamic controller and mixer, stream_handle, and player.
        let (controller, mixer) = mixer::mixer(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(44_100).unwrap(),
        );

        // Create four unique sources. The frequencies used here correspond
        // notes in the key of C and in octave 4: C4, or middle C on a piano,
        // E4, G4, and A4 respectively.
        let source_c = SineWave::new(261.63)
            .take_duration(NOTE_DURATION)
            .amplify(NOTE_AMPLITUDE);
        let source_e = SineWave::new(329.63)
            .take_duration(NOTE_DURATION)
            .amplify(NOTE_AMPLITUDE);
        let source_g = SineWave::new(392.0)
            .take_duration(NOTE_DURATION)
            .amplify(NOTE_AMPLITUDE);
        let source_a = SineWave::new(440.0)
            .take_duration(NOTE_DURATION)
            .amplify(NOTE_AMPLITUDE);

        // Add sources C, E, G, and A to the mixer controller.
        controller.add(source_c);
        controller.add(source_e);
        controller.add(source_g);
        controller.add(source_a);

        info!("beep with position: {:?}", position);
        let queued_source = QueuedAudioSource {
            source: mixer,
            total_duration: Some(Duration::from_secs_f32(1.)),
        }
        .seek_or_skip_duration(position);

        Ok(queued_source)
    }));
}
