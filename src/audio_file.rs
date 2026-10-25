use bevy_asset::{Asset, AssetLoader, LoadContext, io::Reader};
use bevy_reflect::TypePath;
use std::io;
use std::sync::Arc;

//==================================================================================================
// AudioFile
//==================================================================================================

#[derive(Asset, Debug, Clone, TypePath)]
pub struct AudioFile {
    pub bytes: Arc<[u8]>,
}

impl AsRef<[u8]> for AudioFile {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

//==================================================================================================
// AudioFileLoader
//==================================================================================================

#[derive(Default, TypePath)]
pub struct AudioFileLoader;

impl AssetLoader for AudioFileLoader {
    type Asset = AudioFile;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<AudioFile, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(AudioFile {
            bytes: bytes.into(),
        })
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
