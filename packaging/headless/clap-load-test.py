#!/usr/bin/env python3
"""CI: loads the CLAP plug-in the way a CLAP host does (dlopen, clap_entry, init, the plug-in factory) and prints its
id and name. Exit code 1 if it cannot be loaded on this system (wrong C library, missing libraries ...).
    python3 clap-load-test.py RadioMoon-MPX-Transmitter.clap
"""
import ctypes
import sys

path = sys.argv[1]
try:
    lib = ctypes.CDLL(path)
except OSError as e:
    print(f"CLAP load FAILED: {e}")
    sys.exit(1)


class Version(ctypes.Structure):
    _fields_ = [("major", ctypes.c_uint32), ("minor", ctypes.c_uint32), ("revision", ctypes.c_uint32)]


INIT = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_char_p)
DEINIT = ctypes.CFUNCTYPE(None)
GET_FACTORY = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_char_p)


class Entry(ctypes.Structure):
    _fields_ = [("version", Version), ("init", INIT), ("deinit", DEINIT), ("get_factory", GET_FACTORY)]


class Descriptor(ctypes.Structure):
    _fields_ = [("version", Version), ("id", ctypes.c_char_p), ("name", ctypes.c_char_p), ("vendor", ctypes.c_char_p)]


COUNT = ctypes.CFUNCTYPE(ctypes.c_uint32, ctypes.c_void_p)
DESC = ctypes.CFUNCTYPE(ctypes.POINTER(Descriptor), ctypes.c_void_p, ctypes.c_uint32)


class Factory(ctypes.Structure):
    _fields_ = [("count", COUNT), ("descriptor", DESC), ("create", ctypes.c_void_p)]


entry = Entry.in_dll(lib, "clap_entry")
print(f"clap_entry: CLAP {entry.version.major}.{entry.version.minor}.{entry.version.revision}")
if not entry.init(path.encode()):
    print("CLAP init FAILED")
    sys.exit(1)
fp = entry.get_factory(b"clap.plugin-factory")
if not fp:
    print("no plug-in factory")
    sys.exit(1)
f = Factory.from_address(fp)
n = f.count(fp)
for i in range(n):
    d = f.descriptor(fp, i).contents
    print(f"plug-in {i}: {d.id.decode()} - {d.name.decode()} ({d.vendor.decode()})")

# an instance, like a host makes it: its extensions (audio ports, the window)
HOST_EXT = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_char_p)
HOST_REQ = ctypes.CFUNCTYPE(None, ctypes.c_void_p)


class Host(ctypes.Structure):
    _fields_ = [("version", Version), ("host_data", ctypes.c_void_p), ("name", ctypes.c_char_p), ("vendor", ctypes.c_char_p),
                ("url", ctypes.c_char_p), ("hversion", ctypes.c_char_p), ("get_extension", HOST_EXT),
                ("request_restart", HOST_REQ), ("request_process", HOST_REQ), ("request_callback", HOST_REQ)]


P_BOOL = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_void_p)
P_VOID = ctypes.CFUNCTYPE(None, ctypes.c_void_p)
P_EXT = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_char_p)


class Plugin(ctypes.Structure):
    _fields_ = [("desc", ctypes.c_void_p), ("plugin_data", ctypes.c_void_p), ("init", P_BOOL), ("destroy", P_VOID),
                ("activate", ctypes.c_void_p), ("deactivate", ctypes.c_void_p), ("start_processing", ctypes.c_void_p),
                ("stop_processing", ctypes.c_void_p), ("reset", ctypes.c_void_p), ("process", ctypes.c_void_p),
                ("get_extension", P_EXT), ("on_main_thread", P_VOID)]


GUI_SUPPORTED = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_char_p, ctypes.c_bool)


class Gui(ctypes.Structure):
    _fields_ = [("is_api_supported", GUI_SUPPORTED)]


keep = [HOST_EXT(lambda h, i: None), HOST_REQ(lambda h: None)]
host = Host(Version(1, 2, 2), None, b"clap-load-test", b"RadioMoon", b"", b"1", keep[0], keep[1], keep[1], keep[1])
CREATE = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(Host), ctypes.c_char_p)
create = ctypes.cast(f.create, CREATE)
pp = create(fp, ctypes.byref(host), f.descriptor(fp, 0).contents.id) if n else None
if not pp:
    print("CLAP create FAILED")
    sys.exit(1)
plug = Plugin.from_address(pp)
plug.init(pp)
ports = plug.get_extension(pp, b"clap.audio-ports")
gp = plug.get_extension(pp, b"clap.gui")
print(f"extensions: audio-ports {'yes' if ports else 'NO'}, gui {'yes' if gp else 'NO'}")
if gp:
    g = Gui.from_address(gp)
    print("window (floating):", "can open here" if g.is_api_supported(pp, b"x11", True) else "no Transmitter program or web page on this computer")
plug.destroy(pp)
if not ports or not gp:
    sys.exit(1)
entry.deinit()
print("CLAP load OK" if n else "CLAP: no plug-ins")
sys.exit(0 if n else 1)
