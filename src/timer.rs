use std::time::{Duration, Instant};
use rand::Rng;
use tokio::time::sleep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerType {
    Election,
    Heartbeat,
}

pub struct Timer {
    election_timeout: Duration,
    heartbeat_timeout: Duration,
    start_time: Option<Instant>,
    is_running: bool,
    timer_type: TimerType,
}

impl Timer {
        pub fn new_election_timer() -> Self {
        Self {
            election_timeout: Self::random_election_timeout(),
            heartbeat_timeout: Duration::from_millis(100),
            start_time: None,
            is_running: false,
            timer_type: TimerType::Election,
        }
    }

    pub fn new_heartbeat_timer() -> Self {
        Self {
            election_timeout: Duration::from_millis(1500),
            heartbeat_timeout: Duration::from_millis(100),
            start_time: None,
            is_running: false,
            timer_type: TimerType::Heartbeat,
        }
    }

    fn random_election_timeout() -> Duration {
        let mut rng = rand::rng();
        let timeout_ms = rng.random_range(150..300);
        Duration::from_millis(timeout_ms)
    }

    pub fn reset(&mut self) {
        if self.timer_type == TimerType::Election {
            self.election_timeout = Self::random_election_timeout();
        }
        self.start();
    }

    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
        self.is_running = true;
    }

    pub fn stop(&mut self) {
        self.is_running = false;
        self.start_time = None;
    }

    pub fn is_expired(&self) -> bool {
        if !self.is_running {
            return false;
        }
        if let Some(start_time) = self.start_time {
            match self.timer_type { 
                TimerType::Election => {
                    start_time.elapsed() >= self.election_timeout
                }
                TimerType::Heartbeat => {
                    start_time.elapsed() >= self.heartbeat_timeout
                }
            }
        } else {
            false
        }
    }

    pub fn remaining_time(&self) -> Option<Duration> {
        if !self.is_running {
            return None;
        }
        
        if let Some(start_time) = self.start_time {
            let elapsed = start_time.elapsed();
            let timeout = match self.timer_type {
                TimerType::Election => self.election_timeout,
                TimerType::Heartbeat => self.heartbeat_timeout,
            };
            
            if elapsed >= timeout {
                Some(Duration::ZERO)
            } else {
                Some(timeout - elapsed)
            }
        } else {
            None
        }
    }

    pub fn get_timeout(&self) -> Duration {
        match self.timer_type {
            TimerType::Election => self.election_timeout,
            TimerType::Heartbeat => self.heartbeat_timeout,
        }
    }

    fn get_timeout_duration(&self) -> Duration {
        match self.timer_type {
            TimerType::Election => self.election_timeout,
            TimerType::Heartbeat => self.heartbeat_timeout,
        }
    }

    pub async fn wait_for_expiry(&mut self) {
        let timeout_duration = self.get_timeout_duration();
        sleep(timeout_duration).await;
    }

    pub async fn wait_for_expiry_with_timeout(&mut self, max_wait: Duration) -> bool{

        let timeout_duration = self.get_timeout_duration();
       let actual_wait = std::cmp::min(timeout_duration, max_wait);
       sleep(actual_wait).await;
       actual_wait >= timeout_duration
    }

    pub async fn wait_until_expired(&self) {
        if let Some(start_time) = self.start_time {
            let elapsed = start_time.elapsed();
            let timeout_duration = self.get_timeout_duration();
            
            if elapsed < timeout_duration {
                sleep(timeout_duration - elapsed).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[test]
    fn test_timer_creation() {
        let election_timer = Timer::new_election_timer();
        assert_eq!(election_timer.timer_type, TimerType::Election);
        assert!(!election_timer.is_running);

        let heartbeat_timer = Timer::new_heartbeat_timer();
        assert_eq!(heartbeat_timer.timer_type, TimerType::Heartbeat);
        assert!(!heartbeat_timer.is_running);
    }

    #[test]
    fn test_timer_start_stop() {
        let mut timer = Timer::new_election_timer();
        
        assert!(!timer.is_running);
        timer.start();
        assert!(timer.is_running);
        timer.stop();
        assert!(!timer.is_running);
    }

    #[test]
    fn test_timer_not_expired_when_stopped() {
        let mut timer = Timer::new_heartbeat_timer();
        timer.start();
        timer.stop();
        assert!(!timer.is_expired());
    }

    #[test]
    fn test_heartbeat_timeout() {
        let mut timer = Timer::new_heartbeat_timer();
        timer.start();
        
        // Should not be expired immediately
        assert!(!timer.is_expired());
        
        // Wait for timeout
        std::thread::sleep(Duration::from_millis(150));
        assert!(timer.is_expired());
    }

    #[test]
    fn test_election_timeout_randomness() {
        let timer1 = Timer::new_election_timer();
        let timer2 = Timer::new_election_timer();
        let timer3 = Timer::new_election_timer();
        
        // Timeouts should be different (random)
        let timeouts = vec![
            timer1.election_timeout,
            timer2.election_timeout,
            timer3.election_timeout,
        ];
        
        // At least two should be different (very likely)
        assert!(timeouts.iter().any(|&t| t != timeouts[0]));
    }

    #[test]
    fn test_timer_reset() {
        let mut timer = Timer::new_election_timer();
        let original_timeout = timer.election_timeout;
        
        timer.start();
        timer.reset();
        
        // Should have new random timeout
        assert_ne!(timer.election_timeout, original_timeout);
        assert!(timer.is_running);
    }

    #[test]
    fn test_remaining_time() {
        let mut timer = Timer::new_heartbeat_timer();
        
        // Not running
        assert_eq!(timer.remaining_time(), None);
        
        timer.start();
        let remaining = timer.remaining_time().unwrap();
        
        // Should be close to heartbeat timeout
        assert!(remaining <= Duration::from_millis(100));
        assert!(remaining > Duration::from_millis(0));
    }

    #[tokio::test]
    async fn test_async_wait_for_expiry() {
        let mut timer = Timer::new_heartbeat_timer();
        let start = Instant::now();
        
        timer.wait_for_expiry().await;
        
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_async_wait_until_expired() {
        let mut timer = Timer::new_heartbeat_timer();
        timer.start();
        
        // Wait a bit, then wait for remaining time
        sleep(Duration::from_millis(50)).await;
        let start = Instant::now();
        timer.wait_until_expired().await;
        
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(50)); // Should wait remaining time
    }

    #[tokio::test]
    async fn test_async_wait_with_timeout() {
        let mut timer = Timer::new_heartbeat_timer();
        let start = Instant::now();
        
        // Wait with shorter timeout
        let expired = timer.wait_for_expiry_with_timeout(Duration::from_millis(50)).await;
        
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(50));
        assert!(!expired); // Should not have fully expired
    }

    #[tokio::test]
    async fn test_async_wait_with_longer_timeout() {
        let mut timer = Timer::new_heartbeat_timer();
        let start = Instant::now();
        
        // Wait with longer timeout
        let expired = timer.wait_for_expiry_with_timeout(Duration::from_millis(200)).await;
        
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(100));
        assert!(expired); // Should have fully expired
    }
}

