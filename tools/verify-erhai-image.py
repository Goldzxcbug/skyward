#!/usr/bin/env python3
"""Independent, read-only check of a Skyward recovery build against its inputs."""
import argparse, hashlib, json, lzma, pathlib, struct, subprocess
p = argparse.ArgumentParser()
for key in ('stock', 'image', 'binary', 'system-init', 'config', 'font', 'mono', 'mono-license'):
    p.add_argument('--' + key, type=pathlib.Path, required=True)
p.add_argument('--sources', type=pathlib.Path, default=pathlib.Path(__file__).with_name('erhai-sources.json'))
a = p.parse_args()
# Verification must remain enabled even if Python was started with -O.
if not __debug__:
    raise SystemExit('Run this verifier without Python optimization (-O).')
sources = json.loads(a.sources.read_text())
expected_stock = sources['stock_recovery']['sha256']
expected_device = sources['device']['ota_device']
assert hashlib.sha256(a.stock.read_bytes()).hexdigest() == expected_stock, 'stock differs from the reviewed source input'
def image(path):
    b = path.read_bytes()
    assert b[:8] == b'ANDROID!', path
    k, r = struct.unpack_from('<II', b, 8)
    assert struct.unpack_from('<I', b, 40)[0] == 4 and k == 0, 'Pad recovery must be a ramdisk-only boot-v4 image'
    at = 4096 + ((k + 4095) // 4096) * 4096
    cpio = subprocess.run(['lz4', '-d', '-c'], input=b[at:at+r], capture_output=True, check=True).stdout
    files = {}; pos = 0
    while pos + 110 <= len(cpio):
        h = cpio[pos:pos+110]
        assert h[:6] in (b'070701', b'070702')
        fields = [int(h[6+i*8:14+i*8], 16) for i in range(13)]
        start = pos+110; name = cpio[start:start+fields[11]-1].decode(); begin = (start+fields[11]+3)&~3
        if name == 'TRAILER!!!': break
        assert name not in files, ('duplicate', name)
        files[name] = (fields[1], fields[2], fields[3], cpio[begin:begin+fields[6]])
        pos = (begin+fields[6]+3)&~3
    assert b[-64:-60] == b'AVBf'
    original, offset, size = struct.unpack_from('>QQQ', b, len(b)-52)
    assert original <= offset and offset+size <= len(b)-64
    vb = b[offset:offset+size]; assert vb[:4] == b'AVB0'
    rollback = struct.unpack_from('>Q', vb, 112)[0], struct.unpack_from('>I', vb, 124)[0]
    return b, files, rollback, vb, original
stock, sf, sr, _, _ = image(a.stock)
b, files, rollback, vb, original = image(a.image)
assert len(stock) == len(b), 'partition size changed'
assert sr == rollback, ('rollback fields changed', sr, rollback)
assert struct.unpack_from('>I', vb, 28)[0] == 0, 'vbmeta is not freshly unsigned'
auth = struct.unpack_from('>Q', vb, 12)[0]; doff, dlen = struct.unpack_from('>QQ', vb, 96)
d = vb[256+auth+doff:256+auth+doff+dlen]
assert struct.unpack_from('>Q', d, 0)[0] == 2
assert struct.unpack_from('>Q', d, 16)[0] == original
assert d[24:56].rstrip(b'\0') == b'sha256'
n, s, h = struct.unpack_from('>III', d, 56)
assert d[132:132+n] == b'recovery'
salt = d[132+n:132+n+s]; digest = d[132+n+s:132+n+s+h]
assert hashlib.sha256(salt+b[:original]).digest() == digest, 'AVB digest does not cover rebuilt image'
def android_elf(data, static=False):
    assert data[:6] == b'\x7fELF\x02\x01', 'not a 64-bit little-endian ELF'
    assert struct.unpack_from('<H', data, 18)[0] == 183, 'not an AArch64 ELF'
    phoff = struct.unpack_from('<Q', data, 32)[0]
    phsize, count = struct.unpack_from('<HH', data, 54)
    for i in range(count):
        at = phoff+i*phsize
        kind = struct.unpack_from('<I', data, at)[0]
        if kind == 1:
            align = struct.unpack_from('<Q', data, at+48)[0]
            assert align >= 16384, 'ELF load alignment is below 16 KiB'
        if static:
            assert kind != 3, 'system-init asks for a dynamic interpreter'

for name, source in {
    'system/bin/recovery': a.binary, 'skyward/system-init': a.system_init,
    'skyward/device.toml': a.config, 'skyward/MiSansVF.ttf': a.font,
    'skyward/KirkMono-Regular.ttf': a.mono, 'skyward/KirkMono-OFL.txt': a.mono_license,
}.items():
    assert files[name][3] == source.read_bytes(), ('input mismatch', name)
    if name.endswith(('recovery', 'system-init')):
        assert files[name][0] & 0o111
        android_elf(files[name][3], static=name.endswith('system-init'))
font = files['skyward/MiSansVF.ttf'][3]
if font.startswith(b'\xfd7zXZ\x00'): font = lzma.decompress(font)
assert font[:4] in (b'\x00\x01\x00\x00', b'OTTO', b'ttcf'), 'not a valid font'
full_font = pathlib.Path(__file__).resolve().parents[1]/'resources/fonts/MiSansVF.ttf'
assert font == full_font.read_bytes(), 'the image must carry the complete reviewed typeface'
for name in ('7za', 'avbctl', 'resetprop', 'unzip', 'zstd', 'skyward'):
    e = files['system/bin/'+name]; assert e[0] & 0o170000 == 0o120000 and e[3] == b'recovery'
init = files['system/etc/init/hw/init.rc'][3].decode()
assert 'import /system/etc/init/skyward-mtp.rc\n' in init
assert '    start phoenix_recovery\n' not in init
assert '    start recovery\n' in init
assert 'service recovery /system/bin/recovery\n    disabled\n' in init
assert 'service phoenix_recovery /system/bin/phoenix_recovery\n    disabled\n' in init
mtp = files['system/etc/init/skyward-mtp.rc'][3].decode(); assert 'sys.usb.config_disable' not in mtp
props = files['prop.default'][3].decode(); assert 'ro.product.device='+expected_device+'\n' in props
assert 'ro.build.version.release=' not in props
changed = {'system/bin/recovery', 'prop.default', 'system/etc/init/hw/init.rc'} | {'system/bin/'+n for n in ('7za','avbctl','resetprop','unzip','zstd')}
for name, value in sf.items():
    if name == 'res/OplusOS' or name.startswith('res/OplusOS/'): continue
    assert name in files, ('missing original entry', name)
    if name not in changed: assert files[name] == value, ('unexpected original file change', name)
assert not any(n == 'twres' or n.startswith('twres/') for n in files)
allowed_new_files = {
    'skyward/system-init', 'skyward/device.toml', 'skyward/MiSansVF.ttf',
    'skyward/KirkMono-Regular.ttf', 'skyward/KirkMono-OFL.txt',
    'system/etc/task_profiles.json', 'system/etc/init/skyward-mtp.rc',
} | {'system/bin/'+n for n in ('7za', 'avbctl', 'resetprop', 'unzip', 'zstd', 'skyward')}
for name, entry in files.items():
    if name not in sf and entry[0] & 0o170000 != 0o040000:
        assert name in allowed_new_files, ('unexpected added file; vendor libraries must come from installed partitions', name)
print('PASS: pinned stock, boot/cpio, fresh AVB digest + stock rollback, original files, all build inputs, AArch64/16KiB/static-init, complete font, identity, startup and MTP')
print('Reference source:', sources['integrated']['repository'], sources['integrated']['commit'])
print('image bytes:', len(b), 'ramdisk bytes:', struct.unpack_from('<I', b, 12)[0], 'entries:',len(files), 'rollback:', rollback)
print('SHA256:', hashlib.sha256(b).hexdigest())
