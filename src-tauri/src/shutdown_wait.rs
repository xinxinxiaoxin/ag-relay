//! 退出与更新共用的等待状态；取消和进度读取不获取网关生命周期锁。
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownKind {
    Exit,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownPhase {
    Waiting,
    Cancelling,
    Committing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownProgress {
    pub operation_id: u64,
    pub kind: ShutdownKind,
    pub phase: ShutdownPhase,
    pub elapsed_ms: u64,
    pub remaining_ms: u64,
    pub remaining_requests: Option<usize>,
}

struct Active {
    id: u64,
    kind: ShutdownKind,
    phase: ShutdownPhase,
    started: Instant,
    timeout: Duration,
    remaining_requests: Option<usize>,
}
#[derive(Default)]
struct State {
    next_id: u64,
    active: Option<Active>,
}
#[derive(Clone, Default)]
pub struct ShutdownWaits(Arc<Mutex<State>>);
impl ShutdownWaits {
    pub fn begin(&self, kind: ShutdownKind) -> Result<ShutdownWait, String> {
        self.begin_with_timeout(kind, Duration::from_secs(600))
    }
    pub fn begin_with_timeout(
        &self,
        kind: ShutdownKind,
        timeout: Duration,
    ) -> Result<ShutdownWait, String> {
        if timeout.is_zero() || timeout > Duration::from_secs(600) {
            return Err("等待预算须大于零且不超过10分钟。".into());
        }
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.is_some() {
            return Err("退出或更新正在等待，请先完成或取消当前等待。".into());
        }
        state.next_id += 1;
        let id = state.next_id;
        state.active = Some(Active {
            id,
            kind,
            phase: ShutdownPhase::Waiting,
            started: Instant::now(),
            timeout,
            remaining_requests: None,
        });
        Ok(ShutdownWait {
            state: self.0.clone(),
            id,
        })
    }
    pub fn progress(&self) -> Option<ShutdownProgress> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.active.as_ref().map(|a| ShutdownProgress {
            operation_id: a.id,
            kind: a.kind,
            phase: a.phase,
            elapsed_ms: a.started.elapsed().as_millis() as u64,
            remaining_ms: a.timeout.saturating_sub(a.started.elapsed()).as_millis() as u64,
            remaining_requests: a.remaining_requests,
        })
    }
    pub fn cancel(&self, operation_id: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let active = state
            .active
            .as_mut()
            .filter(|a| a.id == operation_id)
            .ok_or("等待操作已变化或结束，请刷新后重试。")?;
        if active.phase == ShutdownPhase::Committing {
            return Err("请求已排空，正在确认停机，已不能取消。".into());
        }
        active.phase = ShutdownPhase::Cancelling;
        Ok(())
    }
}

pub struct ShutdownWait {
    state: Arc<Mutex<State>>,
    id: u64,
}
impl ShutdownWait {
    pub fn operation_id(&self) -> u64 {
        self.id
    }
    pub fn timeout(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .as_ref()
            .filter(|a| a.id == self.id)
            .map_or(Duration::ZERO, |a| a.timeout)
    }
    pub fn remaining(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .as_ref()
            .filter(|a| a.id == self.id)
            .map_or(Duration::ZERO, |a| {
                a.timeout.saturating_sub(a.started.elapsed())
            })
    }
    pub fn cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .as_ref()
            .filter(|a| a.id == self.id)
            .is_none_or(|a| a.phase == ShutdownPhase::Cancelling)
    }
    pub fn update_remaining(&self, count: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.as_mut().filter(|a| a.id == self.id) {
            active.remaining_requests = Some(count);
        }
    }
    pub fn claim_commit(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = state
            .active
            .as_mut()
            .filter(|a| a.id == self.id)
            .ok_or("等待操作已结束。")?;
        if active.phase != ShutdownPhase::Waiting || active.started.elapsed() >= active.timeout {
            return Err("等待已取消或超时，未确认停机。".into());
        }
        active.phase = ShutdownPhase::Committing;
        Ok(())
    }
}
impl Drop for ShutdownWait {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.as_ref().is_some_and(|a| a.id == self.id) {
            state.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 取消不等待生命周期锁且旧编号不能取消下一次操作() {
        let state = ShutdownWaits::default();
        let first = state.begin(ShutdownKind::Exit).unwrap();
        assert_eq!(first.timeout(), Duration::from_secs(600));
        first.update_remaining(2);
        assert_eq!(state.progress().unwrap().remaining_requests, Some(2));
        state.cancel(first.operation_id()).unwrap();
        assert!(first.cancelled());
        assert!(first.claim_commit().is_err());
        let old = first.operation_id();
        drop(first);
        let next = state.begin(ShutdownKind::Update).unwrap();
        assert!(state.cancel(old).is_err());
        assert!(!next.cancelled());
    }
    #[test]
    fn 确认停机与取消只能一个成功() {
        let state = ShutdownWaits::default();
        let wait = state.begin(ShutdownKind::Exit).unwrap();
        wait.claim_commit().unwrap();
        assert_eq!(state.progress().unwrap().phase, ShutdownPhase::Committing);
        assert!(state.cancel(wait.operation_id()).is_err());
        assert!(!wait.cancelled());
        drop(wait);
        assert!(state.progress().is_none());
    }
    #[test]
    fn 超时不能续期或获得停机确认() {
        let state = ShutdownWaits::default();
        let wait = state
            .begin_with_timeout(ShutdownKind::Update, Duration::from_millis(1))
            .unwrap();
        std::thread::sleep(Duration::from_millis(3));
        wait.update_remaining(0);
        assert_eq!(wait.remaining(), Duration::ZERO);
        assert!(wait.claim_commit().is_err());
    }
}
