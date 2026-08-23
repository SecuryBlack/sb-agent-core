//! Buffer offline con backoff exponencial. Hoy solo lo tiene OxiPulse; el
//! TODO ya anticipaba que "lo van a querer todos" — FerroSentry y CromoForge
//! (eventos/deploy logs) tienen exactamente el mismo problema: qué hacer con
//! una lectura cuando el endpoint no responde.
//!
//! Genérico sobre `T` — el tipo de snapshot (métricas, eventos de seguridad,
//! lo que sea) es semántica de agente.

use std::collections::VecDeque;
use tracing::warn;

/// Ring buffer que guarda snapshots mientras el destino es inalcanzable.
/// Al llenarse, descarta el más antiguo para hacer sitio al nuevo.
pub struct OfflineBuffer<T> {
    queue: VecDeque<T>,
    max_size: usize,
}

impl<T> OfflineBuffer<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size.min(1024)),
            max_size,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.queue.len() >= self.max_size {
            self.queue.pop_front();
            warn!(max = self.max_size, "buffer full — dropping oldest snapshot");
        }
        self.queue.push_back(item);
    }

    pub fn drain_all(&mut self) -> Vec<T> {
        self.queue.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Backoff exponencial para comprobaciones de conectividad, en "ticks" del
/// intervalo del agente (no en tiempo absoluto) — así cada agente puede tener
/// su propio `interval_secs` sin que este código sepa nada de eso.
pub struct Backoff {
    current_ticks: u64,
    max_ticks: u64,
    countdown: u64,
}

impl Backoff {
    /// El techo son ~30s de espera máxima entre comprobaciones, expresado en
    /// número de ticks de `interval_secs`.
    pub fn new(interval_secs: u64) -> Self {
        let max_ticks = (30 / interval_secs.max(1)).max(1);
        Self {
            current_ticks: 1,
            max_ticks,
            countdown: 0,
        }
    }

    pub fn should_check(&mut self) -> bool {
        if self.countdown == 0 {
            true
        } else {
            self.countdown -= 1;
            false
        }
    }

    pub fn on_failure(&mut self) {
        self.countdown = self.current_ticks;
        self.current_ticks = (self.current_ticks * 2).min(self.max_ticks);
    }

    pub fn on_success(&mut self) {
        self.current_ticks = 1;
        self.countdown = 0;
    }
}

/// Log de cambio de estado online/offline, sin duplicar el warn/info en cada agente.
pub fn log_status_change(was_offline: bool, now_offline: bool, buffered: usize) {
    match (was_offline, now_offline) {
        (false, true) => warn!("destination unreachable — switching to offline mode"),
        (true, false) => tracing::info!(flushing = buffered, "destination reachable — reconnected"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_buffer_drops_oldest_when_full() {
        let mut buf = OfflineBuffer::new(2);
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(buf.drain_all(), vec![2, 3]);
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let mut b = Backoff::new(10); // max_ticks = 3
        assert!(b.should_check());
        b.on_failure();
        assert_eq!(b.current_ticks, 2);
        b.on_failure();
        assert_eq!(b.current_ticks, 3); // capped at max_ticks
        b.on_success();
        assert_eq!(b.current_ticks, 1);
    }
}
