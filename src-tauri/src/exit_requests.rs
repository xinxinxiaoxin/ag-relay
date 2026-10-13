//! 关闭选择与退出抢占的内存状态；不访问 Tauri、网关、文件或系统配置。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// 在交给阻塞线程池之前抢占；资格由实际执行任务持有，不会因等待方取消而提早释放。
#[derive(Default)]
pub struct ExitRequests(Arc<AtomicBool>);

impl ExitRequests {
    pub fn try_begin(&self) -> Result<ExitAttempt, String> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "退出正在进行，请等待当前操作完成。".to_string())?;
        Ok(ExitAttempt(self.0.clone()))
    }
}

pub struct ExitAttempt(Arc<AtomicBool>);

impl Drop for ExitAttempt {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct CloseState {
    next_id: u64,
    pending: Option<u64>,
    responding: bool,
}

/// 关闭事件先保存于此，前端监听建立后仍可读取同一请求。
#[derive(Default)]
pub struct CloseRequests(Mutex<CloseState>);

/// 只触及桌面窗口的边界；托盘可用性必须在响应时重新查询。
pub trait CloseWindow {
    fn tray_available(&self) -> bool;
    fn hide(&self) -> Result<(), String>;
}

impl CloseRequests {
    /// 退出边界由调用方提供；只有等待安全退出成功才消费请求。
    pub async fn quit<F>(&self, request_id: u64, exit: impl FnOnce() -> F) -> Result<(), String>
    where
        F: std::future::Future<Output = Result<(), String>>,
    {
        let response = self.begin_response(request_id)?;
        exit().await?;
        response.complete();
        Ok(())
    }

    pub fn send_to_tray(&self, request_id: u64, window: &impl CloseWindow) -> Result<(), String> {
        let response = self.begin_response(request_id)?;
        if !window.tray_available() {
            return Err("系统托盘不可用，无法隐藏面板；请选择取消或退出。".into());
        }
        window.hide()?;
        response.complete();
        Ok(())
    }

    pub fn cancel(&self, request_id: u64) -> Result<(), String> {
        self.begin_response(request_id)?.complete();
        Ok(())
    }

    fn begin_response(&self, request_id: u64) -> Result<CloseResponse<'_>, String> {
        let mut state = self.0.lock().unwrap();
        if state.pending != Some(request_id) {
            return Err("关闭请求已失效，请重新操作。".into());
        }
        if state.responding {
            return Err("关闭操作正在进行，请等待当前操作完成。".into());
        }
        state.responding = true;
        Ok(CloseResponse {
            requests: self,
            request_id,
        })
    }

    pub fn get(&self) -> Option<u64> {
        self.0.lock().unwrap().pending
    }

    /// 返回请求编号与是否新建；重复系统关窗不产生新请求或新事件。
    pub fn request(&self) -> (u64, bool) {
        let mut state = self.0.lock().unwrap();
        if let Some(id) = state.pending {
            return (id, false);
        }
        state.next_id += 1;
        let id = state.next_id;
        state.pending = Some(id);
        (id, true)
    }
}

/// 只持有响应资格，不持有 mutex；失败或 Future 被取消均保留原请求并解除忙碌态。
struct CloseResponse<'a> {
    requests: &'a CloseRequests,
    request_id: u64,
}

impl CloseResponse<'_> {
    fn complete(self) {
        let mut state = self.requests.0.lock().unwrap();
        if state.pending == Some(self.request_id) {
            state.pending = None;
            state.responding = false;
        }
    }
}

impl Drop for CloseResponse<'_> {
    fn drop(&mut self) {
        let mut state = self.requests.0.lock().unwrap();
        if state.pending == Some(self.request_id) {
            state.responding = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::{pin, Pin};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }
        let waker = Waker::from(Arc::new(NoopWake));
        future.poll(&mut Context::from_waker(&waker))
    }

    #[test]
    fn 同步抢占后未轮询即取消会释放资格() {
        let exits = ExitRequests::default();
        let attempt = exits.try_begin().unwrap();
        let future = async move { drop(attempt) };
        fn assert_send_static<T: Send + 'static>(_: &T) {}
        assert_send_static(&future);
        assert!(exits.try_begin().is_err());
        drop(future);
        assert!(exits.try_begin().is_ok());
    }

    #[test]
    fn 所有退出入口共享抢占资格_拒绝的旧请求不排队() {
        let exits = ExitRequests::default();
        let first = exits.try_begin().unwrap();
        assert!(exits.try_begin().is_err());
        drop(first);
        let retry = exits.try_begin().unwrap();
        assert!(exits.try_begin().is_err());
        drop(retry);
        assert!(exits.try_begin().is_ok());
    }

    #[test]
    fn 退出等待期间不持锁且拒绝重复动作_失败后不排队重试() {
        let requests = CloseRequests::default();
        let (id, _) = requests.request();
        let finish = Cell::new(false);
        let calls = Cell::new(0);
        let mut first = pin!(requests.quit(id, || {
            calls.set(calls.get() + 1);
            std::future::poll_fn(|_| {
                if finish.get() {
                    Poll::Ready(Err("退出已取消".into()))
                } else {
                    Poll::Pending
                }
            })
        }));
        assert_eq!(poll_once(first.as_mut()), Poll::Pending);
        assert_eq!(requests.get(), Some(id));
        assert_eq!(requests.request(), (id, false));
        assert!(requests.cancel(id).is_err());
        assert!(matches!(
            poll_once(pin!(requests.quit(id, || {
                calls.set(calls.get() + 1);
                async { Ok(()) }
            }))),
            Poll::Ready(Err(_))
        ));
        let window = Window {
            tray: true,
            visible: Cell::new(true),
            hide_fails: false,
        };
        assert!(requests.send_to_tray(id, &window).is_err());
        assert!(window.visible.get());
        finish.set(true);
        assert_eq!(
            poll_once(first.as_mut()),
            Poll::Ready(Err("退出已取消".into()))
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(requests.get(), Some(id));
        requests.cancel(id).unwrap();
    }

    #[test]
    fn 排空失败返回真实错误并保留请求供显式重试() {
        let requests = CloseRequests::default();
        let (id, _) = requests.request();
        assert_eq!(
            poll_once(pin!(requests.quit(id, || async { Err("排空超时".into()) }))),
            Poll::Ready(Err("排空超时".into()))
        );
        assert_eq!(requests.get(), Some(id));
        assert_eq!(
            poll_once(pin!(requests.quit(id, || async { Ok(()) }))),
            Poll::Ready(Ok(()))
        );
        assert_eq!(requests.get(), None);
    }

    struct Window {
        tray: bool,
        visible: Cell<bool>,
        hide_fails: bool,
    }

    impl CloseWindow for Window {
        fn tray_available(&self) -> bool {
            self.tray
        }
        fn hide(&self) -> Result<(), String> {
            if self.hide_fails {
                return Err("隐藏窗口失败".into());
            }
            self.visible.set(false);
            Ok(())
        }
    }

    #[test]
    fn 隐藏失败保留请求_重新检查托盘后成功只消费当前请求() {
        let requests = CloseRequests::default();
        let (id, _) = requests.request();
        let failed = Window {
            tray: true,
            visible: Cell::new(true),
            hide_fails: true,
        };
        assert_eq!(
            requests.send_to_tray(id, &failed),
            Err("隐藏窗口失败".into())
        );
        assert_eq!(requests.get(), Some(id));
        assert!(failed.visible.get());
        let missing = Window {
            tray: false,
            visible: Cell::new(true),
            hide_fails: false,
        };
        assert!(requests.send_to_tray(id, &missing).is_err());
        assert!(missing.visible.get());
        assert_eq!(requests.get(), Some(id));
        let restored = Window {
            tray: true,
            visible: Cell::new(true),
            hide_fails: false,
        };
        requests.send_to_tray(id, &restored).unwrap();
        assert!(!restored.visible.get());
        assert_eq!(requests.get(), None);
    }

    #[test]
    fn 过期请求不能隐藏窗口或发起安全退出() {
        let requests = CloseRequests::default();
        let (old, _) = requests.request();
        requests.cancel(old).unwrap();
        let (current, _) = requests.request();
        let window = Window {
            tray: true,
            visible: Cell::new(true),
            hide_fails: false,
        };
        assert!(requests.send_to_tray(old, &window).is_err());
        assert!(window.visible.get());
        assert!(matches!(
            poll_once(pin!(
                requests.quit(old, || async { panic!("旧请求不应触及网关") })
            )),
            Poll::Ready(Err(_))
        ));
        assert_eq!(requests.get(), Some(current));
    }

    #[test]
    fn 等待方取消后仍保留请求并允许再次选择() {
        let requests = CloseRequests::default();
        let (id, _) = requests.request();
        {
            let mut response = pin!(requests.quit(id, std::future::pending));
            assert_eq!(poll_once(response.as_mut()), Poll::Pending);
        }
        assert_eq!(requests.get(), Some(id));
        requests.cancel(id).unwrap();
        assert_eq!(requests.get(), None);
    }

    #[test]
    fn 无托盘时拒绝隐藏且保留原请求() {
        let requests = CloseRequests::default();
        let (id, _) = requests.request();
        let window = Window {
            tray: false,
            visible: Cell::new(true),
            hide_fails: false,
        };
        assert!(requests.send_to_tray(id, &window).is_err());
        assert!(window.visible.get());
        assert_eq!(requests.get(), Some(id));
    }

    #[test]
    fn 取消只消费匹配请求且旧编号不能消费后续请求() {
        let requests = CloseRequests::default();
        let (first, _) = requests.request();
        assert!(requests.cancel(first + 1).is_err());
        assert_eq!(requests.get(), Some(first));
        requests.cancel(first).unwrap();
        assert_eq!(requests.get(), None);
        let (second, is_new) = requests.request();
        assert!(is_new);
        assert_ne!(second, first);
        assert!(requests.cancel(first).is_err());
        assert_eq!(requests.get(), Some(second));
    }

    #[test]
    fn 前端尚未监听时保留请求且重复关窗沿用同一编号() {
        let requests = CloseRequests::default();
        assert_eq!(requests.get(), None);
        let (id, is_new) = requests.request();
        assert!(is_new);
        assert_eq!(requests.get(), Some(id));
        assert_eq!(requests.request(), (id, false));
        assert_eq!(requests.get(), Some(id));
    }
}
