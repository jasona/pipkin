#!/usr/bin/env python3
"""Virtual absolute pointer via uinput (like a VM tablet). Coordinates are global layout pixels.
usage: mouse.py move X Y | click X Y | drag X1 Y1 X2 Y2 | wheel X Y N | dblclick X Y | tripleclick X Y"""
import os, struct, fcntl, time, sys, fcntl
W, H = 2560, 1080
EV_SYN, EV_KEY, EV_REL, EV_ABS = 0, 1, 2, 3
BTN_LEFT = 0x110
REL_HWHEEL, REL_WHEEL = 6, 8
ABS_X, ABS_Y = 0, 1
def _IOW(t, nr, size): return (1 << 30) | (size << 16) | (ord(t) << 8) | nr
def _IO(t, nr): return (ord(t) << 8) | nr
fd = os.open('/dev/uinput', os.O_WRONLY | os.O_NONBLOCK)
for ev in (EV_KEY, EV_ABS, EV_REL):
    fcntl.ioctl(fd, _IOW('U', 100, 4), ev)
fcntl.ioctl(fd, _IOW('U', 101, 4), BTN_LEFT)
fcntl.ioctl(fd, _IOW('U', 103, 4), ABS_X); fcntl.ioctl(fd, _IOW('U', 103, 4), ABS_Y)
fcntl.ioctl(fd, _IOW('U', 102, 4), REL_WHEEL)
absmax = [0] * 64; absmax[ABS_X] = 32767; absmax[ABS_Y] = 32767
dev = struct.pack('80sHHHHi', b'pi-test-pointer', 3, 0x1234, 0x5678, 1, 0) + struct.pack('64i', *absmax) + struct.pack('64i', *([0]*64)) * 3
os.write(fd, dev)
fcntl.ioctl(fd, _IO('U', 1))
time.sleep(1.0)
def emit(t, c, v): os.write(fd, struct.pack('llHHi', 0, 0, t, c, v))
def sync(): emit(EV_SYN, 0, 0)
def move(x, y):
    emit(EV_ABS, ABS_X, int(x / W * 32767)); emit(EV_ABS, ABS_Y, int(y / H * 32767)); sync(); time.sleep(0.03)
def btn(v): emit(EV_KEY, BTN_LEFT, v); sync(); time.sleep(0.03)
def click(x, y): move(x, y); btn(1); btn(0)
a = sys.argv[1:]; cmd = a[0]; n = [float(v) for v in a[1:]]
if cmd == 'move': move(n[0], n[1])
elif cmd == 'click': click(n[0], n[1])
elif cmd == 'dblclick': click(n[0], n[1]); time.sleep(0.05); click(n[0], n[1])
elif cmd == 'tripleclick':
    for _ in range(3): click(n[0], n[1]); time.sleep(0.05)
elif cmd == 'drag':
    move(n[0], n[1]); time.sleep(0.1); btn(1)
    steps = 30
    for i in range(1, steps + 1):
        move(n[0] + (n[2] - n[0]) * i / steps, n[1] + (n[3] - n[1]) * i / steps); time.sleep(0.02)
    time.sleep(0.2); btn(0)
elif cmd == 'wheel':
    move(n[0], n[1]); time.sleep(0.1)
    for _ in range(abs(int(n[2]))): emit(EV_REL, REL_WHEEL, 1 if n[2] > 0 else -1); sync(); time.sleep(0.03)
time.sleep(0.3)
fcntl.ioctl(fd, _IO('U', 2)); os.close(fd)
