#!/usr/bin/env python3
"""CI: opens the CLAP plug-in's window the way a host does (gui.create floating, gui.show) and checks that the
Transmitter window in plug-in mode appears (xdotool), then gui.hide and that it is gone. Needs a display (Xvfb).
    python3 clap-gui-test.py <plugin.clap>
"""
import ctypes
import subprocess
import sys
import time

path = sys.argv[1]
lib = ctypes.CDLL(path)


class Version(ctypes.Structure):
    _fields_ = [("major", ctypes.c_uint32), ("minor", ctypes.c_uint32), ("revision", ctypes.c_uint32)]


class Entry(ctypes.Structure):
    _fields_ = [("version", Version), ("init", ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_char_p)), ("deinit", ctypes.CFUNCTYPE(None)),
                ("get_factory", ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_char_p))]


class Descriptor(ctypes.Structure):
    _fields_ = [("version", Version), ("id", ctypes.c_char_p)]


HOST_EXT = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_char_p)
HOST_REQ = ctypes.CFUNCTYPE(None, ctypes.c_void_p)


class Host(ctypes.Structure):
    _fields_ = [("version", Version), ("host_data", ctypes.c_void_p), ("name", ctypes.c_char_p), ("vendor", ctypes.c_char_p),
                ("url", ctypes.c_char_p), ("hversion", ctypes.c_char_p), ("get_extension", HOST_EXT),
                ("request_restart", HOST_REQ), ("request_process", HOST_REQ), ("request_callback", HOST_REQ)]


class Factory(ctypes.Structure):
    _fields_ = [("count", ctypes.CFUNCTYPE(ctypes.c_uint32, ctypes.c_void_p)),
                ("descriptor", ctypes.CFUNCTYPE(ctypes.POINTER(Descriptor), ctypes.c_void_p, ctypes.c_uint32)),
                ("create", ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(Host), ctypes.c_char_p))]


P = ctypes.c_void_p


class Plugin(ctypes.Structure):
    _fields_ = [("desc", P), ("plugin_data", P), ("init", ctypes.CFUNCTYPE(ctypes.c_bool, P)), ("destroy", ctypes.CFUNCTYPE(None, P)),
                ("activate", P), ("deactivate", P), ("start_processing", P), ("stop_processing", P), ("reset", P), ("process", P),
                ("get_extension", ctypes.CFUNCTYPE(P, P, ctypes.c_char_p)), ("on_main_thread", ctypes.CFUNCTYPE(None, P))]


B = ctypes.c_bool


class Gui(ctypes.Structure):
    _fields_ = [("is_api_supported", ctypes.CFUNCTYPE(B, P, ctypes.c_char_p, B)), ("get_preferred_api", P),
                ("create", ctypes.CFUNCTYPE(B, P, ctypes.c_char_p, B)), ("destroy", ctypes.CFUNCTYPE(None, P)),
                ("set_scale", P), ("get_size", P), ("can_resize", P), ("get_resize_hints", P), ("adjust_size", P),
                ("set_size", P), ("set_parent", P), ("set_transient", P),
                ("suggest_title", ctypes.CFUNCTYPE(None, P, ctypes.c_char_p)),
                ("show", ctypes.CFUNCTYPE(B, P)), ("hide", ctypes.CFUNCTYPE(B, P))]


callbacks = []
keep = [HOST_EXT(lambda h, i: None), HOST_REQ(lambda h: None), HOST_REQ(lambda h: callbacks.append(1))]
host = Host(Version(1, 2, 2), None, b"GUI test host", b"RadioMoon", b"", b"1", keep[0], keep[1], keep[1], keep[2])

entry = Entry.in_dll(lib, "clap_entry")
assert entry.init(path.encode())
fp = entry.get_factory(b"clap.plugin-factory")
f = Factory.from_address(fp)
pp = f.create(fp, ctypes.byref(host), f.descriptor(fp, 0).contents.id)
plug = Plugin.from_address(pp)
plug.init(pp)
g = Gui.from_address(plug.get_extension(pp, b"clap.gui"))


def windows():
    r = subprocess.run(["xdotool", "search", "--name", "RadioMoon MPX Transmitter - plug-in"], capture_output=True, text=True)
    return [w for w in r.stdout.split() if w]


ok = True
print("embedded supported:", g.is_api_supported(pp, b"x11", False), "(expected False)")
print("floating supported:", g.is_api_supported(pp, b"x11", True))
ok &= g.create(pp, b"x11", True)
g.suggest_title(pp, b"GUI test host")
ok &= g.show(pp)
found = []
for _ in range(60):
    time.sleep(0.5)
    found = windows()
    if found:
        break
print("window after show:", found)
ok &= bool(found)
if found:
    subprocess.run(["xdotool", "getwindowname", found[0]])
    subprocess.run(["import", "-window", "root", "clap-gui.png"])
g.hide(pp)
time.sleep(2)
gone = not windows()
print("window gone after hide:", gone)
ok &= gone
g.destroy(pp)
plug.destroy(pp)
print("CLAP GUI OK" if ok else "CLAP GUI FAILED")
sys.exit(0 if ok else 1)
