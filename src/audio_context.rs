use super::audio_player::AudioPlayer;
use bevy_ecs::prelude::*;
use bevy_log::prelude::*;
use rodio::{
    CpalError, DeviceSinkBuilder, MixerDeviceSink,
    cpal::{
        self,
        traits::{DeviceTrait, HostTrait},
    },
    stream::{DeviceSinkConfig, DeviceSinkError},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

//==================================================================================================
// AudioContextConnectionError
//==================================================================================================

#[derive(Debug)]
pub enum AudioContextConnectionError {
    RebuildFailed(String),
    Cpal(CpalError),
    DeviceSinkError(DeviceSinkError),
}

impl AudioContextConnectionError {
    fn default_error_callback(self) {
        error!("AudioContext error:\n\n{:#?}", self);
    }
}

//==================================================================================================
// AudioContextRebuild
//==================================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioContextRebuild {
    Disabled,
    Enabled { timeout: Duration },
}

//==================================================================================================
// AudioContextDevice
//==================================================================================================

#[derive(Clone)]
pub enum AudioContextDevice {
    Default,
    Required(cpal::Device),
    Preferred(cpal::Device),
}

//==================================================================================================
// AudioContextErrorCallback
//==================================================================================================

pub type AudioContextErrorCallback = fn(AudioContextConnectionError);

//==================================================================================================
// AudioContextDescriptor
//==================================================================================================

#[derive(Clone)]
pub struct AudioContextDescriptor {
    pub device: AudioContextDevice,
    pub rebuild: AudioContextRebuild,
    pub error_callback: AudioContextErrorCallback,
    pub connect: bool,
}

impl Default for AudioContextDescriptor {
    fn default() -> Self {
        Self {
            rebuild: AudioContextRebuild::Enabled {
                timeout: Duration::from_secs(1),
            },
            device: AudioContextDevice::Default,
            error_callback: AudioContextConnectionError::default_error_callback,
            connect: true,
        }
    }
}

//==================================================================================================
// AudioContext
//==================================================================================================

#[derive(Resource)]
pub struct AudioContext {
    mixer_device_sink: Option<Mutex<MixerDeviceSink>>,
    desc: AudioContextDescriptor,
    rebuild_timeout: Duration,
    needs_rebuild: Arc<AtomicBool>,
}

impl AudioContext {
    pub(super) fn new(desc: AudioContextDescriptor) -> Self {
        Self {
            mixer_device_sink: None,
            desc,
            rebuild_timeout: Duration::ZERO,
            needs_rebuild: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn set_descriptor_and_queue_rebuild(&mut self, desc: AudioContextDescriptor) {
        self.desc = desc;
        self.needs_rebuild.store(true, Ordering::SeqCst);
    }

    pub fn descriptor(&self) -> &AudioContextDescriptor {
        &self.desc
    }

    pub fn device_sink_config(&self) -> Option<DeviceSinkConfig> {
        self.mixer_device_sink.as_ref().map(|os| {
            let os_guard = os.lock().unwrap();
            *os_guard.config()
        })
    }

    pub(super) fn needs_rebuild(&mut self, delta: Duration) -> bool {
        self.rebuild_timeout = self.rebuild_timeout.saturating_sub(delta);

        match self.desc.rebuild {
            AudioContextRebuild::Disabled => return false,
            AudioContextRebuild::Enabled { .. } => {
                if self.rebuild_timeout > Duration::ZERO {
                    return false;
                }
            }
        }

        self.needs_rebuild.load(Ordering::SeqCst)
    }

    pub(super) fn rebuild(
        &mut self,
        audio_players: &mut Query<&mut AudioPlayer>,
    ) -> Result<(), String> {
        self.needs_rebuild.store(false, Ordering::SeqCst);

        let connect_tmp = self.desc.connect;
        self.disconnect(audio_players);

        let result = if connect_tmp {
            self.connect(audio_players)
        } else {
            Ok(())
        };
        self.desc.connect = connect_tmp;

        if let Err(e) = &result {
            (self.desc.error_callback)(AudioContextConnectionError::RebuildFailed(e.clone()));
        }

        result
    }

    fn connect(&mut self, audio_players: &mut Query<&mut AudioPlayer>) -> Result<(), String> {
        if self.is_connected() {
            return Ok(());
        }

        let mixer_device_sink = match &self.desc.device {
            AudioContextDevice::Default => self.create_mixer_device_sink_from_default_device(),

            AudioContextDevice::Required(device) => {
                self.create_mixer_device_sink_from_required_device(device.clone())
            }

            AudioContextDevice::Preferred(device) => {
                let device = device.clone();
                match self.create_mixer_device_sink_from_required_device(device.clone()) {
                    Ok(mixer_device_sink) => Ok(mixer_device_sink),
                    Err(e) => {
                        match device.description() {
                            Ok(device_description) => {
                                let name = device_description.name();
                                warn!(
                                    "Preferred device '{name}' is unavailable. \
                                     Falling back to default device.\n{device_description:?} \
                                     Error: {e}"
                                );
                            }
                            Err(desc_e) => {
                                warn!(
                                    "Preferred device 'Unknown' is unavailable. \
                                     Falling back to default device. \
                                     Failed to receive device description: {desc_e} \
                                     Error: {e}"
                                );
                            }
                        }

                        self.create_mixer_device_sink_from_default_device()
                    }
                }
            }
        };

        match mixer_device_sink {
            Ok(mut mixer_device_sink) => {
                mixer_device_sink.log_on_drop(false);

                let mixer = mixer_device_sink.mixer();

                for mut audio_player in audio_players {
                    audio_player.connect(mixer);
                }

                self.mixer_device_sink = Some(Mutex::new(mixer_device_sink));
            }

            Err(e) => (self.desc.error_callback)(AudioContextConnectionError::DeviceSinkError(e)),
        }

        // Set timeout
        self.rebuild_timeout = match self.desc.rebuild {
            AudioContextRebuild::Disabled => Duration::ZERO,
            AudioContextRebuild::Enabled { timeout } => timeout,
        };

        self.is_connected()
            .ok_or_else(|| String::from("AudioContext could not connect."))
    }

    fn create_mixer_device_sink_from_default_device(
        &mut self,
    ) -> Result<MixerDeviceSink, DeviceSinkError> {
        DeviceSinkBuilder::from_default_device()
            .and_then(|builder| {
                let rebuild = self.needs_rebuild.clone();
                let error_callback = self.desc.error_callback;

                builder
                    .with_error_callback(move |e| {
                        mixer_device_sink_error_callback(e, rebuild.clone(), error_callback)
                    })
                    .open_stream()
            })
            .or_else(|original_err| {
                let mut devices = match cpal::default_host().output_devices() {
                    Ok(devices) => devices,
                    Err(e) => {
                        (self.desc.error_callback)(AudioContextConnectionError::Cpal(e));
                        return Err(original_err);
                    }
                };

                devices
                    .find_map(|d| self.create_mixer_device_sink_from_required_device(d).ok())
                    .ok_or(original_err)
            })
    }

    fn create_mixer_device_sink_from_required_device(
        &mut self,
        device: cpal::Device,
    ) -> Result<MixerDeviceSink, DeviceSinkError> {
        DeviceSinkBuilder::from_device(device).and_then(|builder| {
            let rebuild = self.needs_rebuild.clone();
            let error_callback = self.desc.error_callback;

            builder
                .with_error_callback(move |e| {
                    mixer_device_sink_error_callback(e, rebuild.clone(), error_callback)
                })
                .open_sink_or_fallback()
        })
    }

    fn disconnect(&mut self, audio_players: &mut Query<&mut AudioPlayer>) {
        if !self.is_connected() {
            return;
        }

        for mut audio_player in audio_players {
            audio_player.disconnect();
        }

        self.mixer_device_sink = None;
    }

    fn is_connected(&self) -> bool {
        self.mixer_device_sink.is_some()
    }
}

//==================================================================================================
// Helper functions
//==================================================================================================

fn mixer_device_sink_error_callback(
    cpal_error: CpalError,
    rebuild: Arc<AtomicBool>,
    error_callback: AudioContextErrorCallback,
) {
    error!("Audio MixerDeviceSink error: {cpal_error:?}");
    info!(
        "Audio MixerDeviceSink considered invalid. \
        Full rebuild may be triggered depending on settings."
    );
    rebuild.store(true, Ordering::SeqCst);
    error_callback(AudioContextConnectionError::Cpal(cpal_error));
}

/*
impl AudioContext {
    pub fn new_speaker(&mut self, desc: SpeakerDescriptor) -> Handle {
        let mut speaker = Speaker::new_disconnected(desc);

        if let Some(output_stream) = &self.output_stream {
            let os_guard = output_stream.lock().unwrap();
            let mixer = os_guard.mixer();
            speaker.connect(mixer);
        }

        self.speakers.insert(speaker)
    }
}
*/
