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
entry.deinit()
print("CLAP load OK" if n else "CLAP: no plug-ins")
sys.exit(0 if n else 1)
