"""Screenshot the imageedit.exe main window via PrintWindow (works even if covered)."""
import ctypes
import ctypes.wintypes as wt
import sys
import time

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32

PW_RENDERFULLCONTENT = 0x2


def find_main_window(pid):
    """Largest visible top-level window of `pid`.

    The process may own helper windows too (Qt's screen-change observer, and —
    when stdout is redirected — a "PseudoConsoleWindow" at 0x0 that comes first
    in the enumeration). Picking the largest one reliably lands on the real
    editor window.
    """
    result = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hwnd, _lparam):
        wpid = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid and user32.IsWindowVisible(hwnd):
            rect = wt.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            area = (rect.right - rect.left) * (rect.bottom - rect.top)
            if area > 0:
                result.append((area, hwnd))
        return True

    user32.EnumWindows(cb, 0)
    if not result:
        return None
    result.sort(reverse=True)
    return result[0][1]


def main():
    import PIL.ImageGrab  # noqa: F401  (ensure pillow present)
    target_pid = int(sys.argv[1])
    out_path = sys.argv[2]
    hwnd = find_main_window(target_pid)
    if not hwnd:
        print("NOT FOUND")
        sys.exit(1)
    time.sleep(0.3)
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w, h = rect.right - rect.left, rect.bottom - rect.top

    from PIL import Image
    hdc = user32.GetWindowDC(hwnd)
    gdi = ctypes.windll.gdi32
    mem = gdi.CreateCompatibleDC(hdc)
    bmp = gdi.CreateCompatibleBitmap(hdc, w, h)
    gdi.SelectObject(mem, bmp)
    ok = user32.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)

    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                    ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                    ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                    ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                    ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                    ("biClrImportant", ctypes.c_uint32)]

    class BITMAPINFO(ctypes.Structure):
        _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", ctypes.c_uint32 * 3)]

    bmi = BITMAPINFO()
    bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.bmiHeader.biWidth = w
    bmi.bmiHeader.biHeight = -h
    bmi.bmiHeader.biPlanes = 1
    bmi.bmiHeader.biBitCount = 32
    bmi.bmiHeader.biCompression = 0
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bmi), 0)
    img = Image.frombuffer("RGBA", (w, h), buf.raw, "raw", "BGRA", 0, 1)
    img.convert("RGB").save(out_path)
    gdi.DeleteObject(bmp)
    gdi.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    print("saved", (w, h), "printwindow_ok", bool(ok))


if __name__ == "__main__":
    main()
