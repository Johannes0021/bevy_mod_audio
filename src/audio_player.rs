use super::{
    audio_file::AudioFile,
    source_recovery::{RecoverableSource, SourceRecovery, SourceRecoveryError},
};
use bevy_asset::prelude::*;
use bevy_ecs::prelude::*;
use bevy_log::prelude::*;
use rodio::{Decoder, Player, Source, mixer::Mixer, source::SeekError};
use std::{collections::VecDeque, io::Cursor, mem, sync::Mutex, time::Duration};

//==================================================================================================
// QueuedAudioSource
//==================================================================================================

pub enum QueuedAudioSource {
    // Mutex because:
    // Component requires Sync.
    // Rodio sources don't require Sync.
    // I don't know of a better way right now.
    Any(Mutex<Box<dyn Source + Send + 'static>>),
    File(Handle<AudioFile>),
}

impl<S> From<Box<S>> for QueuedAudioSource
where
    S: Source + Send + 'static,
{
    fn from(source: Box<S>) -> Self {
        Self::Any(Mutex::new(source))
    }
}

impl From<Handle<AudioFile>> for QueuedAudioSource {
    fn from(source: Handle<AudioFile>) -> Self {
        Self::File(source)
    }
}

//==================================================================================================
// AudioPlayer
//==================================================================================================

#[derive(Component)]
pub struct AudioPlayer {
    player: Option<Player>,
    pending_sources: VecDeque<QueuedAudioSource>,
    source_recoveries: Vec<SourceRecovery>,
    last_connected_position: Duration,
    volume: f32,
    speed: f32,
    is_paused: bool,
    is_paused_by_suspend: bool,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self {
            player: None,
            pending_sources: VecDeque::new(),
            source_recoveries: Vec::new(),
            last_connected_position: Duration::ZERO,
            volume: 1.0,
            speed: 1.0,
            is_paused: false,
            is_paused_by_suspend: false,
        }
    }
}

impl AudioPlayer {
    pub fn new(source: impl Into<QueuedAudioSource>) -> Self {
        Self::default().with_append(source)
    }

    //pub fn new_looped(audio_source: Handle<AudioFile>) -> Self {
    //    Self::default().with_append_audio_source_looped(audio_source)
    //}

    pub(super) fn connect(&mut self, mixer: &Mixer) {
        if self.is_connected() {
            info!("Disconnecting existing player before connecting to new one.");
            self.disconnect();
        }

        let player = Player::connect_new(mixer);

        player.set_volume(self.volume);
        player.set_speed(self.speed);

        let should_play = !self.is_paused && !self.is_paused_by_suspend;
        if should_play != !player.is_paused() {
            if should_play {
                player.play();
            } else {
                player.pause();
            }
        }

        let mut source_recoveries = Vec::with_capacity(self.source_recoveries.len());
        mem::swap(&mut source_recoveries, &mut self.source_recoveries);
        for recovery in source_recoveries.drain(..) {
            match recovery.recover() {
                Ok(source) => {
                    self.pending_sources
                        .push_front(QueuedAudioSource::Any(Mutex::new(source)));
                }
                Err(e) => {
                    let SourceRecoveryError { error, recovery: _ } = e;
                    error!("Failed to recover source: {error}");
                }
            }
        }

        self.player = Some(player);
    }

    pub(super) fn disconnect(&mut self) {
        if !self.is_connected() {
            return;
        }

        self.cleanup_finished_sources();

        // TAKE!!!
        let Some(player) = self.player.take() else {
            return;
        };

        self.is_paused = player.is_paused();
        self.volume = player.volume();
        self.speed = player.speed();
        self.last_connected_position += player.get_pos();
    }

    pub(super) fn is_connected(&self) -> bool {
        self.player.is_some()
    }

    pub(super) fn on_resume(&mut self) {
        if self.is_paused_by_suspend() {
            self.play();
        }
        self.is_paused_by_suspend = false;
        self.cleanup_finished_sources();
    }

    pub(super) fn on_suspend(&mut self) {
        self.cleanup_finished_sources();
        if self.is_playing() {
            self.pause();
            // Must set this flag after calling `self.pause()`,
            // because `pause()` resets it to false.
            self.is_paused_by_suspend = true;
        }
    }

    pub(super) fn update(&mut self, audio_files: &Assets<AudioFile>) {
        self.cleanup_finished_sources();

        let Some(player) = self.player.as_ref() else {
            return;
        };

        while let Some(queued_source) = self.pending_sources.pop_front() {
            let source = match queued_source {
                QueuedAudioSource::Any(source) => Ok(source.into_inner().unwrap()),
                QueuedAudioSource::File(file) => {
                    let Some(audio_file) = audio_files.get(&file) else {
                        self.pending_sources
                            .push_front(QueuedAudioSource::File(file));
                        break;
                    };

                    let cursor = Cursor::new(audio_file.clone());
                    Decoder::new(cursor).map(|d| Box::new(d) as Box<dyn Source + Send + 'static>)
                }
            };

            let source = match source {
                Ok(source) => source,
                Err(e) => {
                    error!("Failed to append source: {e}");
                    continue;
                }
            };

            let (recoverable, recovery) = RecoverableSource::new(source);
            player.append(recoverable);
            self.source_recoveries.push(recovery);
        }
    }

    fn cleanup_finished_sources(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let player_queue_len = player.len();
        debug_assert!(
            self.source_recoveries.len() >= player_queue_len,
            "Player queue sync bug"
        );
        let already_played = self
            .source_recoveries
            .len()
            .saturating_sub(player_queue_len);
        debug_assert!(
            self.source_recoveries.len() >= already_played,
            "Player queue sync bug"
        );
        let to_remove = already_played.min(self.source_recoveries.len());
        if to_remove > 0 {
            self.source_recoveries.drain(0..to_remove);
            self.last_connected_position = Duration::ZERO;
        }
    }

    pub fn with_append(mut self, source: impl Into<QueuedAudioSource>) -> Self {
        self.append(source);
        self
    }

    pub fn append(&mut self, source: impl Into<QueuedAudioSource>) {
        self.pending_sources.push_back(source.into());
    }

    pub fn with_append_iter<I, S>(mut self, sources: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<QueuedAudioSource>,
    {
        self.append_iter(sources);
        self
    }

    pub fn append_iter<I, S>(&mut self, sources: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<QueuedAudioSource>,
    {
        self.pending_sources
            .extend(sources.into_iter().map(Into::into))
    }

    /// Removes all currently loaded `Source`s from the `Player`, and pauses it.
    pub fn clear(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.clear();
        }
        self.source_recoveries.clear();
        self.pending_sources.clear();
        self.pause();
    }

    pub fn len(&self) -> usize {
        if let Some(player) = self.player.as_ref() {
            let player_queue_len = player.len();
            debug_assert!(
                self.source_recoveries.len() >= player_queue_len,
                "Player queue sync bug"
            );
            let already_played = self
                .source_recoveries
                .len()
                .saturating_sub(player_queue_len);
            (self.source_recoveries.len() + self.pending_sources.len())
                .saturating_sub(already_played)
        } else {
            self.source_recoveries.len() + self.pending_sources.len()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn skip_one(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.skip_one();
        }

        if !self.source_recoveries.is_empty() {
            self.source_recoveries.remove(0);
        } else if !self.pending_sources.is_empty() {
            self.pending_sources.remove(0);
        }
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.set_volume(volume);
        self
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(player) = self.player.as_ref() {
            player.set_volume(volume);
        }
        self.volume = volume;
    }

    pub fn volume(&self) -> f32 {
        if let Some(player) = self.player.as_ref() {
            player.volume()
        } else {
            self.volume
        }
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        self.set_speed(speed);
        self
    }

    pub fn set_speed(&mut self, speed: f32) {
        if let Some(player) = self.player.as_ref() {
            player.set_speed(speed);
        }
        self.speed = speed;
    }

    pub fn speed(&self) -> f32 {
        if let Some(player) = self.player.as_ref() {
            player.speed()
        } else {
            self.speed
        }
    }

    pub fn play(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.play();
            self.is_paused = player.is_paused();
        } else {
            self.is_paused = false;
        }

        self.is_paused_by_suspend = false;
    }

    pub fn is_playing(&self) -> bool {
        !self.is_paused()
    }

    pub fn with_pause(mut self) -> Self {
        self.pause();
        self
    }

    pub fn pause(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.pause();
            self.is_paused = player.is_paused();
        } else {
            self.is_paused = true;
        }

        self.is_paused_by_suspend = false;
    }

    pub fn is_paused(&self) -> bool {
        if let Some(player) = self.player.as_ref() {
            player.is_paused()
        } else {
            self.is_paused
        }
    }

    pub fn is_paused_by_suspend(&self) -> bool {
        self.is_paused_by_suspend
    }

    //pub fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
    //    if let Some(player) = self.player.as_ref()
    //        && !player.empty()
    //    {
    //        player.try_seek(position)
    //    } else let Some() {

    //    }
    //}

    pub fn get_position(&mut self) -> Duration {
        self.cleanup_finished_sources();

        if let Some(player) = self.player.as_ref() {
            player.get_pos()
        } else {
            self.last_connected_position
        }
    }
}
