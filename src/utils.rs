use crate::errors::PsqlExporterError;
use std::{error::Error, time::Duration};
use tokio::{
    select,
    signal::unix::{signal, Signal, SignalKind},
    sync::watch,
};
use tracing::{debug, error, info, instrument};
pub type ShutdownReceiver = watch::Receiver<bool>;
pub type ShutdownSender = watch::Sender<bool>;

#[derive(Debug)]
pub struct SignalHandler {
    terminate: Signal,
    interrupt: Signal,
    quit: Signal,
    hangup: Signal,

    shutdown_channel_tx: ShutdownSender,
    shutdown_channel_rx: ShutdownReceiver,
}

impl SignalHandler {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let (shutdown_channel_tx, shutdown_channel_rx) = watch::channel(false);
        let receiver = Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            quit: signal(SignalKind::quit())?,
            hangup: signal(SignalKind::hangup())?,
            shutdown_channel_tx,
            shutdown_channel_rx,
        };

        Ok(receiver)
    }

    pub fn get_rx_channel(&self) -> ShutdownReceiver {
        self.shutdown_channel_rx.clone()
    }

    #[instrument("SignalHandler", skip_all)]
    pub async fn shutdown_on_signal(&mut self) {
        let signal = self.wait_for_signal().await;

        info!(%signal,  "shutting down");
        if let Err(e) = self.shutdown_channel_tx.send(true) {
            error!(error = %e, "can't send shutdown message");
        } else {
            debug!("shutdown message has been sent, waiting until all task stopped");
            self.shutdown_channel_tx.closed().await;
            debug!("shutdown completed");
        }
    }

    async fn wait_for_signal(&mut self) -> &str {
        select! {
            _ = self.terminate.recv() => "TERM",
            _ = self.interrupt.recv() => "INT",
            _ = self.quit.recv() => "QUIT",
            _ = self.hangup.recv() => "HANGUP",
        }
    }
}

pub struct SleepHelper {
    shutdown_channel: ShutdownReceiver,
}

impl SleepHelper {
    pub fn from(shutdown_channel: ShutdownReceiver) -> Self {
        Self { shutdown_channel }
    }

    #[instrument("SleepHandler", skip_all, fields(duration=?sleep_time))]
    pub async fn sleep(&mut self, sleep_time: Duration) -> Result<(), PsqlExporterError> {
        select! {
            _ = self.shutdown_channel.changed() => {
                debug!("shutdown signal has been received");
                return Err(PsqlExporterError::ShutdownSignalReceived);
            },
            _ = tokio::time::sleep(sleep_time) => {
                debug!("timeout has been reached");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sleep_normal_timeout() {
        let (_tx, rx) = watch::channel(false);
        let mut helper = SleepHelper::from(rx);
        let sleep_duration = Duration::from_millis(100);

        select! {
            result = helper.sleep(sleep_duration) => {
                assert!(result.is_ok());
            },
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                panic!("timeout has been reached");
            }
        }
    }

    #[tokio::test]
    async fn test_sleep_shutdown_signal() {
        let (tx, rx) = watch::channel(false);
        let mut helper = SleepHelper::from(rx);
        let sleep_duration = Duration::from_secs(1);

        // Send shutdown signal
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            tx.send(true).unwrap();
        });

        let result = helper.sleep(sleep_duration).await;
        assert!(matches!(
            result,
            Err(PsqlExporterError::ShutdownSignalReceived)
        ));
    }
}
