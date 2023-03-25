use std::{
    error::Error,
    time::{Duration, SystemTime},
};
use tokio::{
    select,
    signal::unix::{signal, Signal, SignalKind},
    sync::watch,
};
use tracing::{debug, error, info};

use crate::errors::PsqlExporterError;

pub type ShutdownReceiver = watch::Receiver<bool>;
pub type ShutdownSender = watch::Sender<bool>;

const MAX_LOOP_SLEEP_TIME: Duration = Duration::from_secs(5);

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

    pub async fn shutdown_on_signal(&mut self) {
        let signal = self.wait_for_signal().await;

        info!("{signal} signal has been received, shutting down");
        if let Err(e) = self.shutdown_channel_tx.send(true) {
            error!("can't send shutdown message: {}", e);
        };

        debug!("shutdown message has been sent, waiting until all task stopped");
        self.shutdown_channel_tx.closed().await;
        info!("shutdown completed");
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

    pub async fn sleep(&mut self, sleep_time: Duration) -> Result<(), PsqlExporterError> {
        let finish_time = SystemTime::now() + sleep_time;
        let mut rest_sleep_time = sleep_time;

        while rest_sleep_time > Duration::from_micros(0) {
            if self.is_shutdown_state() {
                return Err(PsqlExporterError::ShutdownSignalReceived);
            }

            let loop_sleep_time = if rest_sleep_time > MAX_LOOP_SLEEP_TIME {
                MAX_LOOP_SLEEP_TIME
            } else {
                rest_sleep_time
            };

            tokio::time::sleep(loop_sleep_time).await;
            rest_sleep_time = finish_time
                .duration_since(SystemTime::now())
                .unwrap_or(Duration::from_micros(0));
        }

        if !self.is_shutdown_state() {
            Ok(())
        } else {
            Err(PsqlExporterError::ShutdownSignalReceived)
        }
    }

    fn is_shutdown_state(&mut self) -> bool {
        if self.shutdown_channel.has_changed().unwrap_or(true) {
            let msg = self.shutdown_channel.borrow_and_update();
            if *msg {
                debug!("is_shutdown_state: shutdown signal has been received");
                return true;
            }
        }

        false
    }
}
