# Windows Portability Guide

## Problem: "系统找不到指定的文件" on Other Computers

When running `milo-pet.exe` on a different computer, you might see:
> 无法运行 - 系统找不到指定的文件

This happens because Rust's MSVC toolchain links against the Visual C++ runtime dynamically by default.

## Solution: Static CRT Linking

We now build with **static CRT linking** to bundle all required runtime libraries into the executable.

### Build Command

```powershell
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release
```

Or use the provided script:

```powershell
.\build-release.ps1
```

## Dependencies

### Before Fix (Dynamic Linking)
- ❌ Requires Visual C++ Redistributable on target machine
- ❌ Fails if VC++ runtime not installed

### After Fix (Static Linking)
- ✅ No additional runtime requirements
- ✅ Works on any Windows 10/11 (64-bit)
- ✅ Only depends on Windows system DLLs (always present):
  - kernel32.dll
  - user32.dll
  - gdi32.dll
  - ntdll.dll
  - etc.

## Verification

To check if an exe has static CRT linking:

```powershell
# Check dependencies
ldd milo-pet.exe

# Look for these (should NOT appear if statically linked):
# - VCRUNTIME140.dll
# - VCRUNTIME140_1.dll
# - MSVCP140.dll
```

## File Size Impact

Static linking increases executable size:
- Dynamic: ~1-2 MB
- Static: ~5 MB

This is acceptable for a desktop application as it ensures portability.

## Updated Release Package

The release package `release/milo-pet-v0.2.0-windows.zip` has been rebuilt with static CRT linking.

**Note**: Release packages are too large for git (>100MB). Download from GitHub Releases.

## Building for Release

### Local Build
```powershell
# 1. Clean previous build
cargo clean

# 2. Build with static CRT
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release

# 3. Package (optional)
.\build-release.ps1
```

### CI/CD (GitHub Actions)
```yaml
- name: Build with static CRT
  run: |
    $env:RUSTFLAGS = "-C target-feature=+crt-static"
    cargo build --release
```

## Troubleshooting

### Still getting the error?
1. Make sure you're using the updated zip file (not the old one)
2. Verify the exe has static linking:
   ```powershell
   ldd milo-pet.exe | Select-String "vcruntime"
   ```
   Should return nothing

### Build fails with profile-rustflags error?
Don't use `rustflags` in Cargo.toml `[profile]` section - it requires nightly.
Instead, use the environment variable approach shown above.

## Commit History

- `8c91a48`: Disable console window for GUI application
- Static CRT linking added in release build process
