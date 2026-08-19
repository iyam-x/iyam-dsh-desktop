use std::os::windows::process::CommandExt;
use std::process::Child;
use std::sync::Mutex;
use std::sync::LazyLock;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 全局 DSH 子进程句柄，供 on_exit_handler（无 AppHandle）使用。
/// 由 process.rs spawn 时写入，由 kill_dsh_on_exit() / restart_dsh() 读取清理。
pub static DSH_CHILD: LazyLock<Mutex<Option<Child>>> =
    LazyLock::new(|| Mutex::new(None));

/// 杀掉当前持有的 DSH 子进程（含其派生的全部子进程）并清理状态；幂等，无进程时直接返回。
///
/// node 可能派生孙进程（如目录选择 worker）。Rust 的 `child.kill()` 只会终止直接子进程，
/// 留下孤儿。Windows 上改用 `taskkill /T` 递归终止整棵进程树，确保随应用退出一并清理；
/// 其余平台维持直接 kill。
pub fn kill_dsh_on_exit() {
    if let Ok(mut locked) = DSH_CHILD.lock() {
        if let Some(mut child) = locked.take() {
            let pid = child.id();
            #[cfg(windows)]
            {
                let mut k = std::process::Command::new("taskkill");
                k.args(["/F", "/T", "/PID", &pid.to_string()]);
                k.creation_flags(CREATE_NO_WINDOW);
                let _ = k.output();
                let _ = child.kill();
            }
            #[cfg(not(windows))]
            {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
