//! 进程内存读数（只为诊断，不参与任何业务逻辑）。
//!
//! 打开大 PSD 时「每加载一次工作集 +400 MB」的现象，光看代码猜不出来是
//! 真的没释放还是堆把空闲页留着 —— 这里直接问 Windows 要两个数：
//! working set（物理驻留）和 private bytes（提交量，含已被换出的页）。
//! 空闲但未归还的堆页会留在 private 里却掉出 working set，两者一起看就能
//! 区分「泄漏」和「堆缓存」。
//!
//! 不引新依赖：psapi 的 GetProcessMemoryInfo 手写声明即可。

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }

    /// (working set MB, private bytes MB)，取不到时返回 (0, 0)。
    pub fn snapshot() -> (u64, u64) {
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                std::mem::size_of::<ProcessMemoryCounters>() as u32,
            )
        } != 0;
        if !ok {
            return (0, 0);
        }
        const MB: usize = 1024 * 1024;
        (
            (counters.working_set_size / MB) as u64,
            (counters.pagefile_usage / MB) as u64,
        )
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn snapshot() -> (u64, u64) {
        (0, 0)
    }
}

/// 打印一条 `mem` 事件：`{"phase":"ui","event":"mem","tag":"...","ws":N,"priv":N}`。
///
/// 只在设置了 `IMAGEEDIT_MEM=1` 时输出，默认零噪声零开销。
pub(in crate::bridge) fn log(tag: &str) {
    if std::env::var_os("IMAGEEDIT_MEM").is_none() {
        return;
    }
    let (ws, priv_mb) = imp::snapshot();
    println!(
        "{{\"phase\":\"ui\",\"event\":\"mem\",\"tag\":\"{tag}\",\"ws\":{ws},\"priv\":{priv_mb}}}"
    );
}
