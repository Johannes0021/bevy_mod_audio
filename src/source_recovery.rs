use rodio::{ChannelCount, Sample, SampleRate, Source, source::SeekError};
use std::time::Duration;

//==================================================================================================
// SourceRecoveryError
//==================================================================================================

pub struct SourceRecoveryError {
    pub error: crossbeam_channel::TryRecvError,
    pub recovery: SourceRecovery,
}

//==================================================================================================
// SourceRecovery
//==================================================================================================

pub struct SourceRecovery {
    source_rx: crossbeam_channel::Receiver<Box<dyn Source + Send + 'static>>,
}

impl SourceRecovery {
    pub fn recover(self) -> Result<Box<dyn Source + Send + 'static>, SourceRecoveryError> {
        self.source_rx
            .try_recv()
            .map_err(|error| SourceRecoveryError {
                error,
                recovery: self,
            })
    }
}

//==================================================================================================
// RecoverableSource
//==================================================================================================

pub struct RecoverableSource {
    source_tx: crossbeam_channel::Sender<Box<dyn Source + Send + 'static>>,
    inner: Option<Box<dyn Source + Send + 'static>>,
}

impl Drop for RecoverableSource {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take()
            && let Err(e) = self.source_tx.send(inner)
        {
            // Don't konw how to handle this error?
            self.inner = Some(e.0);
        }
    }
}

impl RecoverableSource {
    pub fn new(source: Box<dyn Source + Send + 'static>) -> (Self, SourceRecovery) {
        let (source_tx, source_rx) = crossbeam_channel::unbounded();
        let this = Self {
            inner: Some(source),
            source_tx,
        };
        let recovery = SourceRecovery { source_rx };

        (this, recovery)
    }

    fn inner(&self) -> &dyn Source {
        self.inner.as_ref().expect("Inner should exist until drop")
    }

    fn inner_mut(&mut self) -> &mut dyn Source {
        self.inner.as_mut().expect("Inner should exist until drop")
    }
}

impl Iterator for RecoverableSource {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner_mut().next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner().size_hint()
    }
}

impl Source for RecoverableSource {
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.inner().current_span_len()
    }

    #[inline]
    fn channels(&self) -> ChannelCount {
        self.inner().channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner().sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.inner().total_duration()
    }

    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner_mut().try_seek(pos)
    }
}
