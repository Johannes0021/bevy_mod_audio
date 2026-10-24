use super::audio_source::AudioSource;
use bevy_ecs::prelude::*;
use bevy_log::prelude::*;
use rodio::{Player, Source, mixer::Mixer, source::SeekError};
use std::time::Duration;

//==================================================================================================
// QueuedAudioSource
//==================================================================================================

pub struct QueuedAudioSource<S> {
    pub source: S,
    /// Optional total duration of this source.
    ///
    /// Used to optimize `try_seek` and `skip_duration` operations for looped sources,
    /// and to correctly restore playback when rebuilding or resetting the audio context.
    pub total_duration: Option<Duration>,
}

impl<S> QueuedAudioSource<S>
where
    S: Source,
{
    pub fn total_duration(&self) -> Option<Duration> {
        if let Some(duration) = self.total_duration {
            Some(duration)
        } else {
            self.source.total_duration()
        }
    }

    pub fn seek_or_skip_duration(self, position: Duration) -> Self {
        let clamped_duration = if let Some(total_duration) = self.total_duration() {
            position.min(total_duration)
        } else {
            position
        };

        self.seek_or_skip_duration_looped(clamped_duration)
    }

    pub fn seek_or_skip_duration_looped(mut self, position: Duration) -> Self {
        // --- Attempt 1: Loop-aware seek (position modulo total duration) ---
        if let Some(total_duration) = self.total_duration() {
            let seek_pos = if position > total_duration {
                let target_nanos = position.as_nanos();
                let loop_nanos = total_duration.as_nanos();
                let seek_nanos = target_nanos % loop_nanos;

                const NANOS_PER_SEC: u128 = 1_000_000_000;

                let secs = seek_nanos / NANOS_PER_SEC;
                let nanos = (seek_nanos % NANOS_PER_SEC) as u32;

                match u64::try_from(secs) {
                    Ok(secs_u64) => Duration::new(secs_u64, nanos),
                    Err(_) => {
                        warn!(
                            "Duration exceeds u64::MAX seconds, falling back to f64. \
                             Precision may be lost for very large durations."
                        );

                        let target_secs = position.as_secs_f64();
                        let loop_secs = total_duration.as_secs_f64();
                        let seek_secs = target_secs % loop_secs;

                        Duration::from_secs_f64(seek_secs)
                    }
                }
            } else {
                position
            };

            if self.source.try_seek(seek_pos).is_ok() {
                return self;
            }
        }

        // --- Attempt 2: Direct seek ---
        let Err(seek_err) = self.source.try_seek(position) else {
            return self;
        };

        // --- Attempt 3: Fallback to skip_duration ---
        if position == Duration::ZERO {
            return self;
        }

        warn!(
            "Seek failed at {} seconds, falling back to skip_duration.\n\nError: {:?}",
            position.as_secs_f64(),
            seek_err,
        );

        self.source = self.source.skip_duration(position).into_inner();

        self
    }
}

//==================================================================================================
// QueuedAudioSourceResult
//==================================================================================================

pub type QueuedAudioSourceResult =
    Result<QueuedAudioSource<Box<dyn Source + Send + 'static>>, String>;

//==================================================================================================
// QueuedAudioSourceFromPosition
//==================================================================================================

/// A closure that produces a `QueuedAudioSource`.
/// The `Duration` parameter represents the playback position of the source.
/// Implementers should respect this duration to allow seamless continuation of audio playback
/// after a rebuild.
///
/// # Convenient functions in [`QueuedAudioSource`] to apply or restore the playback position:
/// - [`QueuedAudioSource::seek_or_skip_duration()`]
/// - [`QueuedAudioSource::seek_or_skip_duration_looped()`]
pub type QueuedAudioSourceFromPosition =
    Box<dyn FnMut(Duration) -> QueuedAudioSourceResult + Send + Sync + 'static>;

//==================================================================================================
// AudioPlayer
//==================================================================================================

#[derive(Component)]
pub struct AudioPlayer {
    player: Option<Player>,
    queue: Vec<QueuedAudioSourceFromPosition>,
    try_seek: Duration,
    volume: f32,
    speed: f32,
    is_paused: bool,
    is_paused_by_suspend: bool,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self {
            player: None,
            queue: Vec::new(),
            try_seek: Duration::ZERO,
            volume: 1.0,
            speed: 1.0,
            is_paused: false,
            is_paused_by_suspend: false,
        }
    }
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn connect(&mut self, mixer: &Mixer) {
        if self.is_connected() {
            info!("Disconnecting existing player before connecting to new one.");
            self.disconnect();
        }

        let player = Player::connect_new(mixer);

        if self.is_paused {
            player.pause();
        }
        player.set_volume(self.volume);
        player.set_speed(self.speed);

        for (idx, load_source) in self.queue.iter_mut().enumerate() {
            let position = if idx == 0 {
                self.try_seek
            } else {
                Duration::ZERO
            };

            match load_source(position) {
                Ok(queued_source) => player.append(queued_source.source),
                Err(e) => error!("Failed to load source: {e}"),
            }
        }

        self.player = Some(player);
    }

    pub(super) fn disconnect(&mut self) {
        if !self.is_connected() {
            return;
        }

        self.update();

        // TAKE!!!
        let Some(player) = self.player.take() else {
            return;
        };

        self.is_paused = player.is_paused();
        self.volume = player.volume();
        self.speed = player.speed();
        self.try_seek += player.get_pos();
    }

    fn is_connected(&self) -> bool {
        self.player.is_some()
    }

    pub(super) fn on_resume(&mut self) {
        if self.is_paused_by_suspend() {
            self.play();
        }
        self.is_paused_by_suspend = false;
    }

    pub(super) fn on_suspend(&mut self) {
        if self.is_playing() {
            self.pause();

            // Must set this flag after calling `self.pause()`,
            // because `pause()` resets it to false.
            self.is_paused_by_suspend = true;
        }
    }

    pub(super) fn update(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let player_queue_len = player.len();
        let queue_len = self.queue.len();

        if player_queue_len < queue_len {
            let to_remove = queue_len - player_queue_len;
            self.queue.drain(0..to_remove);
            self.try_seek = Duration::ZERO;
        }
    }

    /// See [`Self::append`] for more info.
    pub fn with_append<F, S>(mut self, load_source: F) -> Self
    where
        F: FnMut(Duration) -> Result<QueuedAudioSource<S>, String> + Send + Sync + 'static,
        S: Source + Send + 'static,
    {
        self.append(load_source);
        self
    }

    /// Appends an audio source builder function to the player's queue.
    /// See [`QueuedAudioSourceFromPosition`] for more info.
    pub fn append<F, S>(&mut self, mut load_source: F)
    where
        F: FnMut(Duration) -> Result<QueuedAudioSource<S>, String> + Send + Sync + 'static,
        S: Source + Send + 'static,
    {
        self.update();

        if let Some(player) = self.player.as_ref() {
            match load_source(Duration::ZERO) {
                Ok(queued_source) => player.append(queued_source.source),
                Err(e) => error!("Failed to load source: {e}"),
            }
        }

        self.queue.push(Box::new(move |position| {
            load_source(position).map(|queued_source| {
                let QueuedAudioSource {
                    source,
                    total_duration,
                } = queued_source;

                QueuedAudioSource {
                    source: Box::new(source) as Box<dyn Source + Send + 'static>,
                    total_duration,
                }
            })
        }));
    }

    pub fn with_append_audio_source(mut self, audio_source: AudioSource) -> Self {
        self.append_audio_source(audio_source);
        self
    }

    pub fn append_audio_source(&mut self, audio_source: AudioSource) {
        self.append(move |position| {
            audio_source.create_decoder().map(|decoder| {
                QueuedAudioSource {
                    source: decoder,
                    total_duration: audio_source.total_duration(),
                }
                .seek_or_skip_duration(position)
            })
        });
    }

    pub fn with_append_audio_source_looped(mut self, audio_source: AudioSource) -> Self {
        self.append_audio_source_looped(audio_source);
        self
    }

    pub fn append_audio_source_looped(&mut self, audio_source: AudioSource) {
        self.append(move |position| {
            audio_source.create_decoder_looped().map(|decoder| {
                QueuedAudioSource {
                    source: decoder,
                    total_duration: audio_source.total_duration(),
                }
                .seek_or_skip_duration_looped(position)
            })
        });
    }

    pub fn clear(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.clear();
        }

        self.queue.clear();

        if self.is_paused() {
            self.is_paused_by_suspend = false;
        }
    }

    pub fn len(&self) -> usize {
        if let Some(player) = self.player.as_ref() {
            player.len()
        } else {
            self.queue.len()
        }
    }

    pub fn is_empty(&self) -> bool {
        if let Some(player) = self.player.as_ref() {
            player.empty()
        } else {
            self.queue.is_empty()
        }
    }

    pub fn skip_one(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.skip_one();
        } else if !self.queue.is_empty() {
            let _ = self.queue.remove(0);
        }
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.set_volume(volume);
        self
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(player) = self.player.as_ref() {
            player.set_volume(volume);
        } else {
            self.volume = volume;
        }
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
        } else {
            self.speed = speed;
        }
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

    pub fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        if let Some(player) = self.player.as_ref() {
            let result = player.try_seek(position);

            if result.is_ok() {
                self.try_seek = Duration::ZERO;
            }

            result
        } else {
            self.try_seek = position;
            Ok(())
        }
    }

    pub fn get_position(&mut self) -> Duration {
        self.update();

        match self.player.as_ref() {
            Some(player) => self.try_seek + player.get_pos(),
            None => self.try_seek,
        }
    }
}
