pub use rodio;

use bevy_ecs::prelude::*;

use audio_context::{AudioContext, AudioContextDescriptor};
use audio_file::{AudioFile, AudioFileLoader};
use audio_player::AudioPlayer;
use bevy_app::prelude::*;
use bevy_asset::prelude::*;
use bevy_log::prelude::*;
use bevy_time::prelude::*;
use bevy_window::AppLifecycle;

pub mod audio_context;
pub mod audio_file;
pub mod audio_player;
pub mod source_recovery;

pub mod prelude {
    pub use super::{
        AudioPlugin, AudioSystems,
        audio_context::{
            AudioContext, AudioContextConnectionError, AudioContextDescriptor, AudioContextDevice,
            AudioContextErrorCallback, AudioContextRebuild,
        },
        audio_file::AudioFile,
        audio_player::{AudioPlayer, QueuedAudioSource},
    };
}

#[derive(SystemSet, Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioSystems;

#[derive(Default)]
pub struct AudioPlugin {
    pub audio_context_descriptor: AudioContextDescriptor,
}

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<AudioFile>()
            .insert_resource(AudioContext::new(self.audio_context_descriptor.clone()))
            .add_systems(PostUpdate, update.in_set(AudioSystems));

        #[cfg(any(feature = "mp3", feature = "flac", feature = "wav", feature = "vorbis"))]
        {
            app.init_asset_loader::<AudioFileLoader>();
        }
    }
}

fn update(
    time: Res<Time>,
    mut audio_context: ResMut<AudioContext>,
    audio_files: Res<Assets<AudioFile>>,
    mut audio_players: Query<&mut AudioPlayer>,
    mut lifecycle_messages: MessageReader<AppLifecycle>,
    mut running: Local<bool>,
    mut log_init_rebuild: Local<bool>,
) {
    let delta = time.delta();
    let last_running = *running;

    for msg in lifecycle_messages.read() {
        match msg {
            AppLifecycle::Suspended => *running = false,
            AppLifecycle::Running => *running = true,
            AppLifecycle::Idle | AppLifecycle::WillSuspend | AppLifecycle::WillResume => (),
        }
    }

    if *running != last_running {
        if *running {
            if let Err(e) = audio_context.connect(&mut audio_players) {
                error!("Failed to connect the audio context: {e}");
            }
            for mut audio_player in &mut audio_players {
                audio_player.on_resume();
            }
        } else {
            for mut audio_player in &mut audio_players {
                audio_player.on_suspend();
            }
            audio_context.disconnect(&mut audio_players);
        }
    }

    if *running && audio_context.needs_rebuild(delta) {
        if *log_init_rebuild {
            info!("About to rebuild the audio context.");
        }
        if let Err(e) = audio_context.rebuild(&mut audio_players) {
            error!("Failed to rebuild the audio context: {e}");
        }
    }

    if !*log_init_rebuild {
        *log_init_rebuild = audio_context.is_connected();
    }

    let mixer = audio_context.mixer();
    for mut audio_player in &mut audio_players {
        if let Some(mixer) = &mixer
            && !audio_player.is_connected()
        {
            audio_player.connect(mixer);
        }
        audio_player.update(&audio_files);
    }
}
