#[cfg(windows)]
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
    // 关键：先把 child 取出并立即释放锁，再执行 kill/wait。
    // 此前在持有 DSH_CHILD 锁期间调用 child.wait()，而守护线程也在持有同一把锁的
    // 情况下执行 c.wait() 等待 node 退出 —— 形成死锁：kill 需拿锁才能杀 node，
    // 而锁被「等待 node 退出」的守护线程占着，node 不死锁不释放。表现为点击「退出」
    // 后主线程卡死、应用不退、托盘无响应。
    let taken = if let Ok(mut locked) = DSH_CHILD.lock() {
        locked.take()
    } else {
        None
    };
    if let Some(mut child) = taken {
        let _pid = child.id();
        #[cfg(windows)]
        {
            taskkill_tree(pid);
            let _ = child.kill();
        }
        #[cfg(not(windows))]
        {
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    // 兜底：守护线程在 start_dsh 成功后立即把 child 从 DSH_CHILD 取走并独占（阻塞
    // 在 c.wait() 等 node 退出），因此退出清理通常拿不到 child 句柄，仅靠上面会漏杀
    // node。node 的 PID 始终写在 <home>/.iyam-dsh.pid（本次 spawn 或上次会话残留都成立），
    // 按 PID 递归杀整棵进程树兜底。本函数在 .iyam-dsh.pid 被删除之前调用，文件必然可读。
    #[cfg(windows)]
    {
        if let Ok(pid_str) =
            std::fs::read_to_string(crate::installer::dsh_home().join(".iyam-dsh.pid"))
        {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if pid != 0 {
                    taskkill_tree(pid);
                }
            }
        }
    }
}

/// Windows：`taskkill /F /T` 递归终止 pid 整棵进程树，无窗口静默。
#[cfg(windows)]
fn taskkill_tree(pid: u32) {
    let mut k = std::process::Command::new("taskkill");
    k.args(["/F", "/T", "/PID", &pid.to_string()]);
    k.creation_flags(CREATE_NO_WINDOW);
    let _ = k.output();
}
