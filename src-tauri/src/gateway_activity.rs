//! 活动事件主动通知服务。只读文件，公开接缝可在临时目录脱离 Tauri 运行。
use crate::gateway_diagnostics_reader::{rotated_path_of, TailReader};
use notify::{EventKind, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityNotice {
    pub revision: u64,
    pub status: String,
    pub message: Option<String>,
}

enum Wake {
    Changed(Result<(), String>),
    Stop,
}

pub struct ActivitySubscription {
    stop: Option<Sender<Wake>>,
    worker: Option<JoinHandle<()>>,
}

impl ActivitySubscription {
    pub fn start(
        live: Option<PathBuf>,
        sink: impl Fn(ActivityNotice) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_while(live, || true, sink)
    }

    /// 调用方一旦失去路径归属证明，下次文件通知先撤销订阅，不再读取该路径。
    pub fn start_while(
        live: Option<PathBuf>,
        still_known: impl Fn() -> bool + Send + 'static,
        sink: impl Fn(ActivityNotice) + Send + 'static,
    ) -> Result<Self, String> {
        let Some(live) = live.filter(|_| still_known()) else {
            sink(ActivityNotice {
                revision: 0,
                status: "unavailable".into(),
                message: Some(
                    "无法确认当前实例的诊断文件路径，未启用实时监听；仅可手动查看本软件留存记录。"
                        .into(),
                ),
            });
            return Ok(Self {
                stop: None,
                worker: None,
            });
        };
        let parent = live.parent().ok_or("诊断目录无法确定。")?;
        // 首次启动可能还没有 logs；最多监听已知目录的直接父目录，不猜测其它实例路径。
        let directory = if parent.is_dir() {
            parent
        } else {
            parent
                .parent()
                .filter(|p| p.is_dir())
                .ok_or("诊断目录尚不可访问，请重试。")?
        };
        let (tx, rx) = mpsc::channel();
        let changes = tx.clone();
        let target = live.clone();
        let rotated = rotated_path_of(&live);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
                Ok(event)
                    if !matches!(event.kind, EventKind::Access(_))
                        && (event.need_rescan()
                            || event.paths.iter().any(|p| {
                                p == &target || p == &rotated || target.starts_with(p)
                            })) =>
                {
                    let _ = changes.send(Wake::Changed(Ok(())));
                }
                Err(_) => {
                    let _ = changes.send(Wake::Changed(Err("诊断文件通知失败，请重试。".into())));
                }
                _ => {}
            })
            .map_err(|_| "无法建立诊断文件通知，请检查目录权限后重试。")?;
        watcher
            .watch(directory, RecursiveMode::Recursive)
            .map_err(|_| "无法监听诊断目录，请检查目录权限后重试。")?;

        // 先监听，再在捕获长度内建立快照；初始化期间的追加仍在通知队列里，之后补读。
        let mut reader = TailReader::new(live);
        let initial = reader.snapshot();
        let mut initialized = initial.is_ok();
        let mut notice = ActivityNotice {
            revision: 0,
            status: if initialized { "watching" } else { "error" }.into(),
            message: initial.err(),
        };
        sink(notice.clone());
        let worker = thread::Builder::new()
            .name("activity-watch".into())
            .spawn(move || {
                let _watcher = watcher;
                while let Ok(Wake::Changed(mut notification)) = rx.recv() {
                    // 仅在收到通知后合并短突发；不是固定间隔轮询。持续写入也最多等60ms。
                    let until = std::time::Instant::now() + std::time::Duration::from_millis(60);
                    while std::time::Instant::now() < until {
                        match rx.recv_timeout(
                            until.saturating_duration_since(std::time::Instant::now()),
                        ) {
                            Ok(Wake::Stop) => return,
                            Ok(Wake::Changed(result)) if result.is_err() => notification = result,
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    if !still_known() {
                        notice.status = "unavailable".into();
                        notice.message =
                            Some("实例归属已变化，无法确认诊断路径，已撤销实时监听。".into());
                        sink(notice);
                        break;
                    }
                    let result = notification.and_then(|()| {
                        if initialized {
                            reader.poll()
                        } else {
                            reader.snapshot().map(|_| {
                                initialized = true;
                                0
                            })
                        }
                    });
                    match result {
                        Ok(added) => {
                            if added == 0 && notice.status == "watching" {
                                continue;
                            }
                            notice.revision = notice.revision.saturating_add(added as u64);
                            notice.status = "watching".into();
                            notice.message = None;
                        }
                        Err(error) => {
                            if notice.message.as_ref() == Some(&error) {
                                continue;
                            }
                            notice.status = "error".into();
                            notice.message = Some(error);
                        }
                    }
                    sink(notice.clone());
                }
            })
            .map_err(|_| "无法启动活动事件读取任务。")?;
        Ok(Self {
            stop: Some(tx),
            worker: Some(worker),
        })
    }
}

impl Drop for ActivitySubscription {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(Wake::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Tauri 薄适配：由后端受管句柄确认路径，不相信前端传路径/实例归属。
#[derive(Default)]
pub struct ActivityState {
    subscription: std::sync::Mutex<Option<(String, ActivitySubscription)>>,
}

#[derive(Clone, Serialize)]
struct ActivityPayload {
    subscription: String,
    #[serde(flatten)]
    notice: ActivityNotice,
}

#[tauri::command]
pub fn watch_activity_events(
    app: tauri::AppHandle,
    subscription: String,
    state: tauri::State<'_, ActivityState>,
) -> Result<ActivityNotice, String> {
    use tauri::{Emitter, Manager};
    let mut guard = state.subscription.lock().unwrap_or_else(|e| e.into_inner());
    // 先结束旧订阅，避免两条流竞争同一前端。
    guard.take();
    let managed = app
        .state::<crate::gateway_commands::GatewayState>()
        .holds_managed();
    let live = if managed {
        Some(crate::gateway_diagnostics_reader::resolve_diagnostics_paths(&app)?.0)
    } else {
        None
    };
    let initial = std::sync::Arc::new(std::sync::Mutex::new(ActivityNotice {
        revision: 0,
        status: "unavailable".into(),
        message: None,
    }));
    let latest = initial.clone();
    let id = subscription.clone();
    let owner = app.clone();
    let watcher = ActivitySubscription::start_while(
        live,
        move || {
            owner
                .state::<crate::gateway_commands::GatewayState>()
                .holds_managed()
        },
        move |notice| {
            *latest.lock().unwrap_or_else(|e| e.into_inner()) = notice.clone();
            let _ = app.emit(
                "activity-changed",
                ActivityPayload {
                    subscription: id.clone(),
                    notice,
                },
            );
        },
    )?;
    *guard = Some((subscription, watcher));
    let result = initial.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(result)
}

#[tauri::command]
pub fn stop_activity_watch(subscription: String, state: tauri::State<'_, ActivityState>) {
    let mut guard = state.subscription.lock().unwrap_or_else(|e| e.into_inner());
    // 旧组件异步 cleanup 不得关闭新组件的订阅。
    if guard.as_ref().is_some_and(|(id, _)| id == &subscription) {
        guard.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, sync::mpsc, time::Duration};

    const LINE: &str = "{\"event\":\"gateway_started\",\"time\":\"2026-10-12T00:00:00Z\"}\n";

    fn directory(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ag-relay-activity-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn append(path: &std::path::Path, content: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
    }

    #[test]
    fn 快照与订阅交接期间写入不会遗漏() {
        let dir = directory("handoff");
        let live = dir.join("diagnostics.jsonl");
        fs::write(&live, LINE).unwrap();
        let writing = live.clone();
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start(Some(live), move |notice| {
            if notice.revision == 0 {
                append(&writing, LINE);
            }
            tx.send(notice).unwrap();
        })
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().revision, 0);
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap().revision, 1);
        drop(subscription);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 实例归属失去后撤销监听_不误报别的实例活动() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let dir = directory("ownership");
        let live = dir.join("diagnostics.jsonl");
        fs::write(&live, LINE).unwrap();
        let known = Arc::new(AtomicBool::new(true));
        let guard = known.clone();
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start_while(
            Some(live.clone()),
            move || guard.load(Ordering::SeqCst),
            move |notice| {
                let _ = tx.send(notice);
            },
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        known.store(false, Ordering::SeqCst);
        append(&live, LINE);
        let withdrawn = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            (withdrawn.status.as_str(), withdrawn.revision),
            ("unavailable", 0)
        );
        drop(subscription);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 路径不确定不监听也不猜应用目录() {
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start(None, move |notice| {
            let _ = tx.send(notice);
        })
        .unwrap();
        let initial = rx.recv().unwrap();
        assert_eq!(initial.status, "unavailable");
        assert!(initial.message.unwrap().contains("未启用实时监听"));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(subscription);
    }

    #[test]
    fn 读取错误明确通知_恢复后历史不算新增() {
        let dir = directory("recovery");
        let live = dir.join("diagnostics.jsonl");
        fs::create_dir(&live).unwrap();
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start(Some(live.clone()), move |notice| {
            let _ = tx.send(notice);
        })
        .unwrap();
        let error = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(error.status, "error");
        assert!(error.message.unwrap().contains("读取诊断记录失败"));
        fs::remove_dir(&live).unwrap();
        fs::write(&live, LINE).unwrap();
        let recovered = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(recovered.status, "watching");
        // 文件恢复前不存在与恢复后快照都可以形成有效基线；后续追加必须继续通知。
        append(&live, LINE);
        let change = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(change.revision > recovered.revision);
        drop(subscription);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 文件通知_轮转后重新查询保留筛选页码且不重复() {
        use crate::gateway_diagnostics_reader::{query_activity_in, ActivityQuery, LevelFilter};
        let dir = directory("rotation");
        let live = dir.join("diagnostics.jsonl");
        fs::write(&live, LINE).unwrap();
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start(Some(live.clone()), move |notice| {
            let _ = tx.send(notice);
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        append(&live, LINE);
        fs::rename(&live, rotated_path_of(&live)).unwrap();
        fs::write(&live, LINE.repeat(2)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        let mut revision = 0;
        while revision < 3 {
            let notice = rx
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap();
            assert_eq!(notice.status, "watching");
            assert!(notice.revision > revision);
            revision = notice.revision;
        }
        assert_eq!(revision, 3);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        let page = query_activity_in(
            &live,
            &rotated_path_of(&live),
            &ActivityQuery {
                level: LevelFilter::Info,
                keyword: "网关".into(),
                page: 1,
                page_size: 2,
            },
        )
        .unwrap();
        assert_eq!((page.page, page.events.len(), page.has_more), (1, 2, false));
        drop(subscription);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 文件通知_首次不报历史_追加后主动通知可重新查询() {
        let dir = directory("append");
        let live = dir.join("diagnostics.jsonl");
        fs::write(&live, LINE).unwrap();
        let (tx, rx) = mpsc::channel();
        let subscription = ActivitySubscription::start(Some(live.clone()), move |notice| {
            let _ = tx.send(notice);
        })
        .unwrap();
        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!((first.revision, first.status.as_str()), (0, "watching"));
        append(&live, LINE);
        let change = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("新记录应主动通知，不需要调用poll或切页");
        assert_eq!(change.revision, 1, "历史不计入新增");
        assert!(
            rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "同一追加不能重报"
        );
        drop(subscription);
        fs::remove_dir_all(dir).unwrap();
    }
}
