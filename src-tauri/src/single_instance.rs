//! 串行化单实例插件初始化，并保留主窗口就绪前收到的唤起请求。
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
};

/// 持有到 Builder::build 返回：此时官方插件已创建通知窗口。
/// Windows mutex 由取得它的线程释放；HANDLE 不实现 Send，避免跨线程析构。
#[must_use = "启动锁必须持有到单实例插件完成初始化"]
pub struct StartupGuard {
    #[cfg(windows)]
    handle: HANDLE,
}

impl StartupGuard {
    pub fn acquire(identifier: &str) -> io::Result<Self> {
        #[cfg(windows)]
        {
            if identifier.is_empty() || identifier.contains('\0') {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "无效的应用标识"));
            }
            // 与插件的 {identifier}-sim 独立；所有启动都先等待，不能只检查是否已存在。
            let name: Vec<u16> = format!("Local\\{identifier}-ag-relay-startup-gate")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let error = match unsafe { WaitForSingleObject(handle, 10_000) } {
                // 插件在第二进程内直接 process::exit，跳过 Rust Drop；下一个等待者
                // 会取得 abandoned mutex。这里没有共享数据需要恢复，可继续初始化。
                WAIT_OBJECT_0 | WAIT_ABANDONED => return Ok(Self { handle }),
                WAIT_TIMEOUT => io::Error::new(io::ErrorKind::TimedOut, "等待应用启动锁超过 10 秒"),
                _ => io::Error::last_os_error(),
            };
            unsafe { CloseHandle(handle) };
            Err(error)
        }
        #[cfg(not(windows))]
        {
            let _ = identifier;
            Ok(Self {})
        }
    }
}

impl Drop for StartupGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

/// 返回 true 的调用者负责显示主窗口；Ready 前的多个请求可合并为一次唤起。
#[derive(Default)]
pub struct ActivationRequests {
    ready: AtomicBool,
    pending: AtomicBool,
}

impl ActivationRequests {
    pub fn request(&self) -> bool {
        self.pending.store(true, Ordering::SeqCst);
        self.ready.load(Ordering::SeqCst) && self.pending.swap(false, Ordering::SeqCst)
    }

    pub fn mark_ready(&self) -> bool {
        // 先发布 Ready，再取走 pending；与 request 交错时总有一方接走待办。
        self.ready.store(true, Ordering::SeqCst);
        self.pending.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::ActivationRequests;

    #[test]
    fn 主窗口就绪前的重复启动在就绪后补唤起() {
        let activation = ActivationRequests::default();
        assert!(!activation.request());
        assert!(!activation.request());
        assert!(activation.mark_ready());
        assert!(!activation.mark_ready());
        assert!(activation.request());
    }

    #[test]
    fn 无重复启动时不额外唤起_就绪后每次请求即时唤起() {
        let activation = ActivationRequests::default();
        assert!(!activation.mark_ready());
        assert!(activation.request());
        assert!(activation.request());
    }
}
