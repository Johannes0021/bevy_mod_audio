use bevy_asset::{Asset, AssetLoader, LoadContext, io::Reader};
use bevy_reflect::TypePath;
use rodio::{
    ChannelCount, Decoder, SampleRate, Source,
    decoder::{DecoderBuilder, LoopedDecoder},
};
use std::{
    io::{self, Cursor},
    sync::Arc,
    time::Duration,
};

//==================================================================================================
// AudioSource
//==================================================================================================

#[derive(Asset, TypePath, Debug, Clone)]
pub struct AudioSource {
    bytes: Arc<[u8]>,
    total_duration: Option<Duration>,
    sample_rate: SampleRate,
    channels: ChannelCount,
}

impl AsRef<[u8]> for AudioSource {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AudioSource {
    pub fn try_from_bytes(bytes: Arc<[u8]>) -> Result<Self, io::Error> {
        let cursor = Cursor::new(bytes.clone());
        let mut builder = DecoderBuilder::new().with_data(cursor);

        if let Ok(len) = u64::try_from(bytes.len()) {
            builder = builder.with_byte_len(len).with_seekable(true);
        }

        match builder.build() {
            Ok(decoder) => Ok(Self {
                bytes,
                total_duration: decoder.total_duration(),
                sample_rate: decoder.sample_rate(),
                channels: decoder.channels(),
            }),
            Err(e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to create decoder!\nError: {e}"),
            )),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    pub fn channels(&self) -> ChannelCount {
        self.channels
    }

    pub fn create_decoder(&self) -> Result<Decoder<Cursor<Arc<[u8]>>>, String> {
        let cursor = Cursor::new(self.bytes.clone());
        let mut builder = DecoderBuilder::new().with_data(cursor);

        if let Ok(len) = u64::try_from(self.bytes.len()) {
            builder = builder.with_byte_len(len).with_seekable(true);
        }

        builder
            .build()
            .map_err(|e| format!("Failed to create decoder!\nError: {e}"))
    }

    pub fn create_decoder_looped(&self) -> Result<LoopedDecoder<Cursor<Arc<[u8]>>>, String> {
        let cursor = Cursor::new(self.bytes.clone());
        let mut builder = DecoderBuilder::new().with_data(cursor);

        if let Ok(len) = u64::try_from(self.bytes.len()) {
            builder = builder.with_byte_len(len).with_seekable(true);
        }

        builder
            .build_looped()
            .map_err(|e| format!("Failed to create looped decoder!\nError: {e}"))
    }
}

//==================================================================================================
// AudioLoader
//==================================================================================================

#[derive(Default, TypePath)]
pub struct AudioLoader;

impl AssetLoader for AudioLoader {
    type Asset = AudioSource;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<AudioSource, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        AudioSource::try_from_bytes(bytes.into())
    }

    fn extensions(&self) -> &[&str] {
        &[
            #[cfg(feature = "mp3")]
            "mp3",
            #[cfg(any(feature = "flac", feature = "symphonia-flac"))]
            "flac",
            #[cfg(any(feature = "wav", feature = "symphonia-wav"))]
            "wav",
            #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
            "oga",
            #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
            "ogg",
            #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
            "spx",
        ]
    }
}
