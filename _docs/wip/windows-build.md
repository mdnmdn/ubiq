---
id: wip-windows-build
title: Windows build FAQ
kind: wip
status: current
summary: "Answers for the failures a Windows (GNU toolchain) build hits that macOS never does — the one found so far is a stale dlltool.exe on PATH breaking raw-dylib import-lib generation, fixed by an environment change, never by a code change."
read_when: you are building this tree on Windows with the GNU toolchain and hit a linker or import-lib error
updated: 2026-09-07
---

# Windows build FAQ

Windows is not the primary platform (macOS is), so the Cargo build rarely runs here, and any
failure that only shows up on Windows gets answered in this file, one entry per issue, as it is
found and fixed. Entries cover **environment** fixes only: a build problem fixed in code is not
recorded here, because the diff itself documents it.

## Q: `failed to add native library …\bcryptprimitives.dll_imports.lib`

`cargo build` does not finish while compiling `getrandom` (0.4.x) with:

```text
error: failed to add native library C:\…\target\debug\deps\rustcXXXX\bcryptprimitives.dll_imports.lib: The system cannot find the file specified. (os error 2)
```

**Cause.** On the GNU toolchain rustc generates raw-dylib import libraries (here, for
`bcryptprimitives.dll`) by invoking the `dlltool.exe` it finds first on `PATH`. On the machine that
hit this, a stale `C:\Users\<user>\.cargo\bin\dlltool.exe` (dated 2025-02-02) shadows the real
MinGW-w64 binary: it is itself a broken build that exits with `STATUS_DLL_NOT_FOUND` (0xC0000135)
and prints nothing. rustc sees empty stderr, treats the run as a success, and then cannot open the
`.lib` the run should have produced. The failure reproduces in a minimal two-file crate, with
`-j1`, and in any project — it is not a parallelism race and not workspace-specific.

### Why does the failure name getrandom

`getrandom` on Windows draws randomness from `bcryptprimitives`, and it is the first crate in this
tree to need a raw-dylib import library. The error names that first victim, but the root cause is
the tool, not the crate: any later raw-dylib import would fail the same way.

### How to confirm it is the stale dlltool

Run `dlltool --version` from the failing shell. A broken shim prints nothing at all; the real
MinGW-w64 binary prints `GNU … (Binutils for MinGW-W64 …)`. `git status` stays clean throughout,
because the problem is in the toolchain discovery path, not in the tree.

### The fix

Rename or delete the stale shim so PATH lookup falls through to the real MinGW-w64 `dlltool`, then
rebuild:

```powershell
Rename-Item "$HOME\.cargo\bin\dlltool.exe" "dlltool.exe.broken.bak"
```

The import library is then generated in the proper target directory and the build proceeds. The
rename is applied and reversibly kept on the machine that hit the failure.

## Q: The error names a different `.dll`

The crate varies with whichever raw-dylib import comes first, so a message wearing `ntdll`, `user32`
or another system library on it means the same thing. The root cause and the fix are unchanged: the
named `.lib` is only the first one the broken `dlltool` failed to produce.

## Q: Does the fix need a commit

No. `git status` is clean after it: the fix is a rename in `~/.cargo`, outside this tree, and no
manifest or dependency changed. A peer building on macOS, or on a Windows machine that never had the
stale shim, needs nothing from it.

## Q: What if PATH resolution picks another dlltool

PATH order decides. The `.cargo\bin` directory sits ahead of the MinGW-w64 `bin`, which is why the
stale shim won. Removing it promotes the first healthy `dlltool` on PATH; whether that is the
WinLibs one or a different toolchain's does not matter, as long as it prints a version and produces
the `.lib`. Confirm with `dlltool --version` before and after.

## Q: Does anything here affect macOS

No. The `dlltool` import-lib path is MinGW-only; the macOS toolchain never invokes it (on POSIX,
rustc builds no import libraries). macOS stays the reference build, and this FAQ gains no macOS
entries unless a macOS-only failure reappears — the code that needed `#[cfg(unix)]` gating around
`SIGWINCH` keeps its POSIX behaviour there and is therefore not an environment item.