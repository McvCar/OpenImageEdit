"""Reproduce a real mouse drag on the imageedit window via PostMessage
(window-targeted, NOT SendInput — safe while the user is working).

Posts WM_LBUTTONDOWN / WM_MOUSEMOVE / WM_LBUTTONUP directly to the app's
main HWND. Takes PrintWindow screenshots mid-drag so the frozen-preview
bug can be observed exactly as the user sees it.

Usage: post_drag.py <pid> <out_prefix> [x0 y0 x1 y1]
  x0..y1: physical screen coords of the drag (default: auto-detect the
          image rect from a screenshot and drag corner to corner).
"""
import ctypes
import ctypes.wintypes as wt
import sys
import time

u = ctypes.windll.user32
g = ctypes.windll.gdi32
k = ctypes.windll.kernel32
u.SetProcessDPIAware()

WM_MOUSEMOVE = 0x0200
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_MOUSEACTIVATE = 0x0021
MK_LBUTTON = 0x0001
PW_RENDERFULLCONTENT = 0x2


def find_main_window(pid):
    result = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hwnd, _lparam):
        wpid = wt.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid and u.IsWindowVisible(hwnd):
            result.append(hwnd)
        return True

    u.EnumWindows(cb, 0)
    return result[0] if result else None


def client_size(hwnd):
    r = wt.RECT()
    u.GetClientRect(hwnd, ctypes.byref(r))
    return r.right - r.left, r.bottom - r.top


def screenshot(hwnd, path):
    w, h = client_size(hwnd)
    hdc = u.GetWindowDC(hwnd)
    mem = g.CreateCompatibleDC(hdc)
    bmp = g.CreateCompatibleBitmap(hdc, w, h)
    g.SelectObject(mem, bmp)
    u.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)

    class BMIH(ctypes.Structure):
        _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                    ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                    ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                    ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                    ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                    ("biClrImportant", ctypes.c_uint32)]

    bmi = BMIH(ctypes.sizeof(BMIH), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(w * h * 4)
    g.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bmi), 0)
    from PIL import Image
    img = Image.frombuffer("RGBA", (w, h), buf.raw, "raw", "BGRA", 0, 1)
    img.save(path)
    g.DeleteObject(bmp)
    g.DeleteDC(mem)
    u.ReleaseDC(hwnd, hdc)
    return img


def detect_image_rect(img):
    """Bounding box of the colorful (saturated) region = the loaded image."""
    from PIL import Image as I
    rgb = img.convert("RGB")
    px = rgb.load()
    minx, miny, maxx, maxy = 10**9, 10**9, -1, -1
    for y in range(0, rgb.height, 4):
        for x in range(0, rgb.width, 4):
            r, gr, b = px[x, y]
            mx, mn = max(r, gr, b), min(r, gr, b)
            if mx > 60 and (mx - mn) > 40:  # saturated pixel
                if x < minx: minx = x
                if y < miny: miny = y
                if x > maxx: maxx = x
                if y > maxy: maxy = y
    if maxx < 0:
        return None
    return (minx, miny, maxx, maxy)


def post(hwnd, msg, x, y, wParam=0):
    lparam = (int(y) << 16) | (int(x) & 0xFFFF)
    u.PostMessageW(hwnd, msg, wParam, lparam)


def main():
    pid = int(sys.argv[1])
    prefix = sys.argv[2]
    hwnd = find_main_window(pid)
    if not hwnd:
        print("NO-WINDOW")
        sys.exit(1)
    print("hwnd", hwnd)
    time.sleep(0.3)

    base = screenshot(hwnd, prefix + "-t0.png")
    if len(sys.argv) >= 6:
        x0, y0, x1, y1 = map(int, sys.argv[3:7])
    else:
        r = detect_image_rect(base)
        if not r:
            print("NO-IMAGE-RECT")
            sys.exit(1)
        minx, miny, maxx, maxy = r
        # A few px inside the corners. Client coords = screenshot coords
        # minus the non-client top/left; we post CLIENT coords, and the
        # screenshot starts at the window origin. Measure the offset:
        # PrintWindow content starts at the window frame; client origin
        # is offset by the border/title. Detect via window rect vs client.
        wr = wt.RECT()
        u.GetWindowRect(hwnd, ctypes.byref(wr))
        pt = wt.POINT(0, 0)
        u.ClientToScreen(hwnd, ctypes.byref(pt))
        ox, oy = pt.x - wr.left, pt.y - wr.top
        x0, y0 = minx + 2 - ox, miny + 2 - oy
        x1, y1 = maxx - 2 - ox, maxy - 2 - oy
    print("drag", x0, y0, "->", x1, y1)

    # Wake the hover pipeline.
    for _ in range(3):
        post(hwnd, WM_MOUSEMOVE, x0, y0)
        time.sleep(0.03)

    post(hwnd, WM_LBUTTONDOWN, x0, y0, MK_LBUTTON)
    time.sleep(0.12)

    steps = 24
    shots = {5: "s5", 12: "s12", 20: "s20"}
    for i in range(1, steps + 1):
        x = x0 + (x1 - x0) * i // steps
        y = y0 + (y1 - y0) * i // steps
        post(hwnd, WM_MOUSEMOVE, x, y, MK_LBUTTON)
        if i in shots:
            time.sleep(0.15)
            screenshot(hwnd, f"{prefix}-{shots[i]}.png")
            print("shot", shots[i], "at", x, y)
        time.sleep(0.04)

    time.sleep(0.12)
    post(hwnd, WM_LBUTTONUP, x1, y1, 0)
    time.sleep(0.4)
    screenshot(hwnd, prefix + "-end.png")
    print("DONE")


if __name__ == "__main__":
    main()
