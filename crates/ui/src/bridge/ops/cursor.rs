//! 画布光标：按住 Shift 显示「加选」，按住 Alt 显示「减选」，按住 Ctrl 停在
//! 选区上显示「剪刀」（剪切并拖动选区内容）。
//!
//! 为什么绕到 Win32：
//!   - Qt 内置的 CursorShape 里没有 +/−/剪刀，`Qt.DragCopyCursor` 在 Windows
//!     上只会退化成箭头；
//!   - QML 没法构造自定义光标（只有 MouseArea/HoverHandler 的枚举属性），
//!     cxx-qt-lib 也没绑 QCursor。
//!
//! 所以这里直接自己画三枚 32×32 光标（黑线 + 白色阴影，深浅照片上都看得见），
//! 再给每个顶层窗口挂一层 WndProc 子类化。**必须拦 WM_SETCURSOR**：Qt 在那
//! 个消息里无条件 SetCursor 它自己的光标，光调一次 SetCursor 会被鼠标移动时
//! 的下一帧盖掉，表现为「剪刀闪一下就没了」。

use super::super::*;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

impl qobject::Bridge {
    /// 0 = 交回 Qt（跟随 cursorShape），1 = 加选(+)，2 = 减选(−)，3 = 剪刀，
    /// 4 = 图层旋转，5 = 吸管（修复画笔 / 画笔按住 Alt 取样）。
    /// QML 侧的 `cursorKind` 绑定变化时调用；重复值会被忽略。
    pub fn cursor_set_kind(self: Pin<&mut Self>, kind: i32) {
        set_kind(kind);
    }

    /// 把当前线程的顶层窗口挂上 WndProc（幂等，新开窗口后再调一次即可）。
    /// QML 在窗口首次可见时调用。
    pub fn cursor_install(self: Pin<&mut Self>) {
        install();
    }

    /// 修饰键状态：bit0 Shift / bit1 Ctrl / bit2 Alt。
    /// QML 拿不到「只按下修饰键、鼠标没动」的事件，只能靠悬停期间轮询这里。
    pub fn modifier_bits(self: Pin<&mut Self>) -> i32 {
        modifier_bits()
    }
}

// ---------------------------------------------------------------- 公共状态 --

/// 当前画布期望的光标种类（0 = 不接管）。
static KIND: AtomicI32 = AtomicI32::new(0);

/// 接管前的系统光标：从剪刀切回十字时用它恢复，免得要等鼠标动一下才变。
static PREV_CURSOR: Mutex<isize> = Mutex::new(0);

/// 已创建的光标句柄（下标 = 种类）。
static CURSORS: OnceLock<[isize; 6]> = OnceLock::new();

/// 已子类化的窗口：(hwnd, 原 WndProc)。
static HOOKED: Mutex<Vec<(isize, isize)>> = Mutex::new(Vec::new());

fn set_kind(kind: i32) {
    let kind = kind.clamp(0, 5);
    let prev = KIND.swap(kind, Ordering::Relaxed);
    if prev == kind {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use win::*;
        if kind > 0 {
            if let Some(&cursor) = CURSORS.get().map(|list| &list[kind as usize]) {
                if prev == 0 {
                    // 记下 Qt 正在用的光标，退出接管时原样还回去。
                    if let Ok(mut slot) = PREV_CURSOR.lock() {
                        *slot = GetCursor();
                    }
                }
                SetCursor(cursor);
            }
        } else if prev > 0 {
            let restore = PREV_CURSOR.lock().map(|s| *s).unwrap_or(0);
            if restore != 0 {
                SetCursor(restore);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = prev;
}

fn modifier_bits() -> i32 {
    #[cfg(windows)]
    unsafe {
        // GetKeyState 读的是**调用线程**的按键队列，必须在 GUI 线程上调 ——
        // QML 的 Timer 正好跑在 GUI 线程。
        let down = |vk: i32| (win::GetKeyState(vk) as u16 & 0x8000) != 0;
        let mut bits = 0;
        if down(0x10) {
            bits |= 1; // VK_SHIFT
        }
        if down(0x11) {
            bits |= 2; // VK_CONTROL
        }
        if down(0x12) {
            bits |= 4; // VK_MENU (Alt)
        }
        bits
    }
    #[cfg(not(windows))]
    0
}

/// 建光标 + 给线程里所有顶层窗口挂 WndProc（幂等）。
fn install() {
    #[cfg(windows)]
    unsafe {
        let _ = CURSORS.get_or_init(build_cursors);
        let mut handles: Vec<isize> = Vec::new();
        win::EnumThreadWindows(
            win::GetCurrentThreadId(),
            Some(collect_window),
            &mut handles as *mut Vec<isize> as isize,
        );
        for hwnd in handles {
            if HOOKED.lock().map(|list| list.iter().any(|(h, _)| *h == hwnd)).unwrap_or(true) {
                continue;
            }
            let old = win::SetWindowLongPtrW(hwnd, GWLP_WNDPROC, window_proc as isize);
            if old != 0 {
                if let Ok(mut list) = HOOKED.lock() {
                    list.push((hwnd, old));
                }
            }
        }
    }
}

#[cfg(windows)]
extern "system" fn collect_window(hwnd: isize, param: isize) -> i32 {
    // SAFETY: param 是 install() 栈上 Vec 的裸指针，枚举期间一直有效。
    let list = unsafe { &mut *(param as *mut Vec<isize>) };
    list.push(hwnd);
    1 // 继续枚举
}

#[cfg(windows)]
const GWLP_WNDPROC: i32 = -4;
#[cfg(windows)]
const WM_SETCURSOR: u32 = 0x0020;
#[cfg(windows)]
const WM_NCDESTROY: u32 = 0x0082;
#[cfg(windows)]
const HTCLIENT: i32 = 1;

#[cfg(windows)]
extern "system" fn window_proc(hwnd: isize, msg: u32, w: usize, l: isize) -> isize {
    use win::*;
    // SAFETY: 这是 Win32 窗口过程回调 —— 参数由系统传入（hwnd 是有效窗口句柄，
    // prev 是我们自己从 SetWindowLongPtrW 拿到并存在 HOOKED 里的原过程地址），
    // 所有调用都只是把消息转交给原始过程/设置光标。
    unsafe {
        if msg == WM_SETCURSOR && (l & 0xFFFF) as i32 == HTCLIENT {
            let kind = KIND.load(Ordering::Relaxed);
            if kind > 0 {
                if let Some(&cursor) = CURSORS.get().map(|list| &list[kind as usize]) {
                    if cursor != 0 {
                        SetCursor(cursor);
                        return 1; // TRUE：光标已设好，别交给 Qt 再覆盖
                    }
                }
            }
        }
        let prev = HOOKED
            .lock()
            .ok()
            .and_then(|list| list.iter().find(|(h, _)| *h == hwnd).map(|(_, p)| *p))
            .unwrap_or(0);
        if msg == WM_NCDESTROY {
            // 窗口没了：把原 WndProc 还回去，别让我们的函数指针跟着悬空。
            if let Ok(mut list) = HOOKED.lock() {
                list.retain(|(h, _)| *h != hwnd);
            }
            if prev != 0 {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev);
            }
            return DefWindowProcW(hwnd, msg, w, l);
        }
        if prev == 0 {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        CallWindowProcW(prev, hwnd, msg, w, l)
    }
}

// ---------------------------------------------------------------- 画光标 --

#[cfg(windows)]
const CURSOR_SIZE: usize = 32;
#[cfg(windows)]
type Pixels = [[f32; 4]; CURSOR_SIZE * CURSOR_SIZE];

/// 五枚光标 + 一个空位（下标 0 不用）。建不出来就退化成「不接管」。
#[cfg(windows)]
fn build_cursors() -> [isize; 6] {
    let mut out = [0isize; 6];
    let mut buf: Pixels = [[0.0; 4]; CURSOR_SIZE * CURSOR_SIZE];
    // 加选：十字 + 右下角 "+"
    paint_crosshair(&mut buf);
    paint_plus(&mut buf);
    out[1] = unsafe { create_cursor(&buf, 16, 16) }.unwrap_or(0);
    // 减选：十字 + 右下角 "−"
    reset(&mut buf);
    paint_crosshair(&mut buf);
    paint_minus(&mut buf);
    out[2] = unsafe { create_cursor(&buf, 16, 16) }.unwrap_or(0);
    // 剪刀：热点在刀轴（两片刀的交点），拖动时手感居中
    reset(&mut buf);
    paint_scissors(&mut buf);
    out[3] = unsafe { create_cursor(&buf, 16, 16) }.unwrap_or(0);
    // 旋转：绕圈的箭头，热点在圆心
    reset(&mut buf);
    paint_rotate(&mut buf);
    out[4] = unsafe { create_cursor(&buf, 16, 16) }.unwrap_or(0);
    // 吸管：热点在管尖（左下），取样时对准的是尖端点到的那个像素。
    reset(&mut buf);
    paint_dropper(&mut buf);
    out[5] = unsafe { create_cursor(&buf, DROPPER_HOT.0, DROPPER_HOT.1) }.unwrap_or(0);
    out
}

#[cfg(windows)]
fn reset(buf: &mut Pixels) {
    *buf = [[0.0; 4]; CURSOR_SIZE * CURSOR_SIZE];
}

/// 往像素上盖一层颜色（straight alpha 的 src-over）。
#[cfg(windows)]
fn plot(buf: &mut Pixels, x: i32, y: i32, rgb: (u8, u8, u8), cov: f32) {
    if x < 0 || y < 0 || x as usize >= CURSOR_SIZE || y as usize >= CURSOR_SIZE {
        return;
    }
    let sa = cov.clamp(0.0, 1.0);
    if sa <= 0.002 {
        return;
    }
    let dst = &mut buf[y as usize * CURSOR_SIZE + x as usize];
    let da = dst[3];
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0001 {
        return;
    }
    let src = [rgb.0 as f32, rgb.1 as f32, rgb.2 as f32];
    for i in 0..3 {
        dst[i] = (src[i] * sa + dst[i] * da * (1.0 - sa)) / (oa * 255.0);
    }
    dst[3] = oa;
}

#[cfg(windows)]
fn dist_to_segment(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0001 {
        0.0
    } else {
        (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (x0 + dx * t, y0 + dy * t);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// 抗锯齿直线：`half` 是半线宽（0.5 ≈ 1px）。
#[cfg(windows)]
fn stroke_line(buf: &mut Pixels, x0: f32, y0: f32, x1: f32, y1: f32, half: f32, rgb: (u8, u8, u8)) {
    let pad = half + 1.0;
    let xa = (x0.min(x1) - pad).floor().max(0.0) as i32;
    let xb = (x0.max(x1) + pad).ceil().min(CURSOR_SIZE as f32) as i32;
    let ya = (y0.min(y1) - pad).floor().max(0.0) as i32;
    let yb = (y0.max(y1) + pad).ceil().min(CURSOR_SIZE as f32) as i32;
    for y in ya..yb {
        for x in xa..xb {
            let d = dist_to_segment(x as f32 + 0.5, y as f32 + 0.5, x0, y0, x1, y1);
            let cov = (half + 0.5 - d).clamp(0.0, 1.0);
            plot(buf, x, y, rgb, cov);
        }
    }
}

/// 抗锯齿圆环（剪刀的两个指环）。
#[cfg(windows)]
fn stroke_ring(buf: &mut Pixels, cx: f32, cy: f32, r: f32, half: f32, rgb: (u8, u8, u8)) {
    let pad = r + half + 1.0;
    let xa = (cx - pad).floor().max(0.0) as i32;
    let xb = (cx + pad).ceil().min(CURSOR_SIZE as f32) as i32;
    let ya = (cy - pad).floor().max(0.0) as i32;
    let yb = (cy + pad).ceil().min(CURSOR_SIZE as f32) as i32;
    for y in ya..yb {
        for x in xa..xb {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let cov = (half + 0.5 - (d - r).abs()).clamp(0.0, 1.0);
            plot(buf, x, y, rgb, cov);
        }
    }
}

#[cfg(windows)]
const INK: (u8, u8, u8) = (18, 18, 20);
#[cfg(windows)]
const HALO: (u8, u8, u8) = (255, 255, 255);

/// 黑线 + 向右下 1px 的白色阴影：浅色照片上黑线可见，深色照片上白影可见。
#[cfg(windows)]
fn shadowed_line(buf: &mut Pixels, x0: f32, y0: f32, x1: f32, y1: f32, half: f32) {
    stroke_line(buf, x0 + 1.0, y0 + 1.0, x1 + 1.0, y1 + 1.0, half, HALO);
    stroke_line(buf, x0, y0, x1, y1, half, INK);
}

#[cfg(windows)]
fn paint_crosshair(buf: &mut Pixels) {
    let c = 16.0;
    // 收短到 7（原来 9）：原来的十字末端正好落在 (24,24) 那个 +/− 徽标
    // 上，两者叠在一起，放大看像同一个符号，实际尺寸下根本分不出加还是减。
    let arm = 7.0;
    shadowed_line(buf, c, c - arm, c, c + arm, 0.5);
    shadowed_line(buf, c - arm, c, c + arm, c, 0.5);
}

#[cfg(windows)]
fn paint_plus(buf: &mut Pixels) {
    // 徽标外移到 (26,26) 并加粗：和十字臂留出空隙，32px 下也能一眼看出。
    let (cx, cy, arm) = (26.0, 26.0, 4.5);
    shadowed_line(buf, cx - arm, cy, cx + arm, cy, 0.8);
    shadowed_line(buf, cx, cy - arm, cx, cy + arm, 0.8);
}

#[cfg(windows)]
fn paint_minus(buf: &mut Pixels) {
    let (cx, cy, arm) = (26.0, 26.0, 4.5);
    shadowed_line(buf, cx - arm, cy, cx + arm, cy, 0.8);
}

/// 剪刀：两片刀从右侧张开，左侧两个指环，交点即热点。
#[cfg(windows)]
fn paint_scissors(buf: &mut Pixels) {
    // 刀身（右上 / 右下），穿过交点延伸到左侧指环。
    shadowed_line(buf, 27.0, 6.0, 8.0, 22.0, 0.9);
    shadowed_line(buf, 27.0, 26.0, 8.0, 10.0, 0.9);
    // 指环：白色阴影先铺，再压黑环。
    for (cx, cy) in [(6.5f32, 25.0f32), (6.5f32, 7.0f32)] {
        stroke_ring(buf, cx + 1.0, cy + 1.0, 3.2, 1.0, HALO);
        stroke_ring(buf, cx, cy, 3.2, 1.0, INK);
    }
    // 刀轴
    stroke_ring(buf, 17.5, 16.5, 0.5, 1.4, HALO);
    stroke_ring(buf, 16.5, 15.5, 0.5, 1.4, INK);
}

/// 旋转：一个留缺口的圆弧 + 缺口端的两根箭头，读起来就是「转一圈」。
#[cfg(windows)]
fn paint_rotate(buf: &mut Pixels) {
    use std::f32::consts::PI;
    let (cx, cy, r) = (16.0f32, 16.5f32, 8.5f32);
    // 从 -140° 顺时针扫到 +125°，缺口留在左侧，箭头挂在弧的末端。
    let (a0, a1) = (-2.44f32, 2.18f32);
    let steps = 30;
    let mut prev = (cx + r * a0.cos(), cy + r * a0.sin());
    for i in 1..=steps {
        let t = a0 + (a1 - a0) * (i as f32) / (steps as f32);
        let p = (cx + r * t.cos(), cy + r * t.sin());
        shadowed_line(buf, prev.0, prev.1, p.0, p.1, 0.8);
        prev = p;
    }
    // 箭头：从弧末端沿着切线**向后**张开两根倒刺，指向前进方向。
    let (ex, ey) = (cx + r * a1.cos(), cy + r * a1.sin());
    for spread in [-0.62f32, 0.62] {
        let back = a1 + PI + spread;
        shadowed_line(
            buf,
            ex,
            ey,
            ex + 5.0 * back.cos(),
            ey + 5.0 * back.sin(),
            0.8,
        );
    }
}

/// 吸管（修复画笔 / 画笔按住 Alt 取样）：斜置的管身 + 右上角的球囊，
/// 热点在左下管尖——按下时对准的就是尖端点到的那个像素。
#[cfg(windows)]
const DROPPER_HOT: (i32, i32) = (6, 27);

#[cfg(windows)]
fn paint_dropper(buf: &mut Pixels) {
    // 管身：先白后黑（shadowed_line 先画右下 1px 白影，浅色照片上也看得见）。
    shadowed_line(buf, 7.0, 26.0, 21.0, 12.0, 1.9);
    // 管尖收窄：一小段更细的线，读起来像"针尖"。
    shadowed_line(buf, 5.5, 27.5, 8.0, 25.0, 1.2);
    // 球囊：白色外圈先铺，再压黑色实心圆。
    fill_disc(buf, 24.5, 7.5, 5.4, HALO);
    fill_disc(buf, 24.5, 7.5, 4.3, INK);
    // 球囊高光（白线压在黑圆左上），32px 下也一眼能看出是吸管而不是圆点。
    stroke_line(buf, 22.2, 6.6, 24.6, 4.6, 0.7, HALO);
}

/// 实心圆：抗锯齿边缘。
#[cfg(windows)]
fn fill_disc(buf: &mut Pixels, cx: f32, cy: f32, r: f32, rgb: (u8, u8, u8)) {
    let xa = (cx - r - 1.0).floor().max(0.0) as i32;
    let xb = (cx + r + 1.0).ceil().min(CURSOR_SIZE as f32) as i32;
    let ya = (cy - r - 1.0).floor().max(0.0) as i32;
    let yb = (cy + r + 1.0).ceil().min(CURSOR_SIZE as f32) as i32;
    for y in ya..yb {
        for x in xa..xb {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let cov = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            plot(buf, x, y, rgb, cov);
        }
    }
}

/// 把 RGBA 缓冲做成一枚 Win32 光标（32bpp 颜色 + 1bpp 透明掩码）。#[cfg(windows)]
unsafe fn create_cursor(buf: &Pixels, hot_x: i32, hot_y: i32) -> Option<isize> {
    use std::ffi::c_void;
    use win::*;

    let screen = GetDC(0);
    if screen == 0 {
        return None;
    }
    let hdc = CreateCompatibleDC(screen);

    let header = |bits: u16| BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width: CURSOR_SIZE as i32,
        // 负高度 = 自上而下，省得写位图时还要翻行。
        height: -(CURSOR_SIZE as i32),
        planes: 1,
        bit_count: bits,
        compression: 0,
        size_image: 0,
        x_pels: 0,
        y_pels: 0,
        clr_used: 0,
        clr_important: 0,
    };
    let mut color_info = BitmapInfo { header: header(32), colors: [0; 256] };
    let mut mask_info = BitmapInfo { header: header(1), colors: [0; 256] };

    let mut color_bits: *mut c_void = std::ptr::null_mut();
    let mut mask_bits: *mut c_void = std::ptr::null_mut();
    let color = CreateDIBSection(hdc, &color_info, 0, &mut color_bits, 0, 0);
    let mask = CreateDIBSection(hdc, &mask_info, 0, &mut mask_bits, 0, 0);
    if color == 0 || mask == 0 || color_bits.is_null() || mask_bits.is_null() {
        if color != 0 {
            DeleteObject(color);
        }
        if mask != 0 {
            DeleteObject(mask);
        }
        DeleteDC(hdc);
        ReleaseDC(0, screen);
        return None;
    }

    // 颜色：预乘 alpha 的 BGRA（Windows 的光标合成按预乘处理）。
    let dst = std::slice::from_raw_parts_mut(color_bits as *mut u8, CURSOR_SIZE * CURSOR_SIZE * 4);
    for (i, px) in buf.iter().enumerate() {
        let a = px[3].clamp(0.0, 1.0);
        dst[i * 4] = (px[2] * a * 255.0).round() as u8;
        dst[i * 4 + 1] = (px[1] * a * 255.0).round() as u8;
        dst[i * 4 + 2] = (px[0] * a * 255.0).round() as u8;
        dst[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    // 掩码：1 = 透明。32px 宽正好 4 字节一行。
    let mask_bytes = CURSOR_SIZE * CURSOR_SIZE / 8;
    let mbits = std::slice::from_raw_parts_mut(mask_bits as *mut u8, mask_bytes);
    for (i, byte) in mbits.iter_mut().enumerate() {
        let mut value = 0u8;
        for bit in 0..8 {
            let index = i * 8 + bit;
            if buf[index][3] < 0.004 {
                value |= 0x80 >> bit;
            }
        }
        *byte = value;
    }

    let mut info = IconInfo { icon: 1, hot_x, hot_y, mask, color };
    let cursor = CreateIconIndirect(&mut info);
    DeleteObject(color);
    DeleteObject(mask);
    DeleteDC(hdc);
    ReleaseDC(0, screen);
    if cursor == 0 {
        None
    } else {
        Some(cursor)
    }
}

// ---------------------------------------------------------------- 调试用 --

/// 把三枚光标渲染成 PNG（探针用：`--script=cursor-dump:C:/tmp/cursor`）。
/// 直接导出绘制缓冲，方便在 Windows 上看清 32×32 的笔画，不用截图抓光标。
#[cfg(windows)]
pub fn dump_cursors(prefix: &str) {
    let mut buf: Pixels = [[0.0; 4]; CURSOR_SIZE * CURSOR_SIZE];
    let jobs: [(&str, fn(&mut Pixels)); 4] = [
        ("add", |b| {
            paint_crosshair(b);
            paint_plus(b)
        }),
        ("sub", |b| {
            paint_crosshair(b);
            paint_minus(b)
        }),
        ("cut", paint_scissors),
        ("pick", paint_dropper),
    ];
    for (name, paint) in jobs {
        reset(&mut buf);
        paint(&mut buf);
        let mut img = image::RgbaImage::new(CURSOR_SIZE as u32, CURSOR_SIZE as u32);
        for (i, px) in buf.iter().enumerate() {
            let x = (i % CURSOR_SIZE) as u32;
            let y = (i / CURSOR_SIZE) as u32;
            let a = px[3].clamp(0.0, 1.0);
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (px[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (px[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (px[2].clamp(0.0, 1.0) * 255.0) as u8,
                    (a * 255.0) as u8,
                ]),
            );
        }
        let path = format!("{prefix}-{name}.png");
        let _ = img.save(&path);
        println!("{{\"phase\":\"ui\",\"event\":\"cursor-dump\",\"path\":\"{}\"}}", path.replace('\\', "/"));
    }
}

#[cfg(not(windows))]
pub fn dump_cursors(_prefix: &str) {}

// ------------------------------------------------------------- Win32 声明 --

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub type Handle = isize;

    #[repr(C)]
    pub struct BitmapInfoHeader {
        pub size: u32,
        pub width: i32,
        pub height: i32,
        pub planes: u16,
        pub bit_count: u16,
        pub compression: u32,
        pub size_image: u32,
        pub x_pels: i32,
        pub y_pels: i32,
        pub clr_used: u32,
        pub clr_important: u32,
    }

    #[repr(C)]
    pub struct BitmapInfo {
        pub header: BitmapInfoHeader,
        pub colors: [u32; 256],
    }

    #[repr(C)]
    pub struct IconInfo {
        pub icon: i32,
        pub hot_x: i32,
        pub hot_y: i32,
        pub mask: Handle,
        pub color: Handle,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn SetCursor(cursor: Handle) -> Handle;
        pub fn GetCursor() -> Handle;
        pub fn CreateIconIndirect(info: *mut IconInfo) -> Handle;
        pub fn GetDC(hwnd: Handle) -> Handle;
        pub fn ReleaseDC(hwnd: Handle, hdc: Handle) -> i32;
        pub fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
        pub fn CallWindowProcW(prev: isize, hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
        pub fn DefWindowProcW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
        pub fn EnumThreadWindows(
            thread: u32,
            callback: Option<extern "system" fn(Handle, isize) -> i32>,
            param: isize,
        ) -> i32;
        pub fn GetKeyState(vk: i32) -> i16;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        pub fn CreateCompatibleDC(hdc: Handle) -> Handle;
        pub fn DeleteDC(hdc: Handle) -> i32;
        pub fn CreateDIBSection(
            hdc: Handle,
            info: *const BitmapInfo,
            usage: u32,
            bits: *mut *mut c_void,
            section: Handle,
            offset: u32,
        ) -> Handle;
        pub fn DeleteObject(obj: Handle) -> i32;
    }
}

