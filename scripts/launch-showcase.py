#!/usr/bin/env python3
# Build and launch the Day showcase on one or more targets.
#
#     scripts/launch-showcase.py macos-appkit
#     scripts/launch-showcase.py desktop
#     scripts/launch-showcase.py macos-appkit web-dom
#     scripts/launch-showcase.py mobile --env DAY_DEMO_ROUTE=canvas
#
# On Windows, invoke it through the interpreter — `python scripts\launch-showcase.py desktop`
# (the `python3` alias there is a Microsoft Store stub that only offers to install Python).
#
# Each argument is either a platform-toolkit name (`macos-appkit`, `windows-xaml`, …) or one of the
# symbolic groups:
#
#     desktop   every desktop target this host can build (macOS: appkit, gtk, qt)
#     mobile    every phone-class target this host can build (iOS, Android, HarmonyOS) — each needs
#               its simulator/emulator already running; `day launch` says which is missing
#     web       web-dom, served over loopback with a browser opened on it
#
# Anything starting with `-` is passed straight through to `day launch`, so `--env K=V`,
# `--locale`, `--script`, and `--detach` all work. `--dry-run` prints the targets an argument list
# expands to and stops, without building anything.
#
# Symlink it anywhere and call it from anywhere — `ln -s .../day/scripts/launch-showcase.py
# ~/bin/showcase`. The script resolves its own path through the link chain, so it always finds the
# checkout it actually lives in, whatever the caller's working directory is.
#
# The app is built in RELEASE: a debug build of a UI framework indicates nothing about what a user
# would experience, and the showcase exists to be looked at. The `day` CLI itself is built from this
# checkout in debug — it is the build tool, not the thing being measured.
#
# Python 3.8+, standard library only.
import json
import os
import subprocess
import sys
from pathlib import Path

# The `▶` below and the `—`/`…` in the header are unencodable on a Windows console still running a
# legacy codepage (cp437, cp1252), where printing them raises UnicodeEncodeError and takes the
# script down before it does anything. Degrade those characters to `?` instead of dying; a terminal
# already on UTF-8 (the Windows Terminal default, and every macOS/Linux one) is unaffected.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(errors="replace")
    except (AttributeError, ValueError):
        pass

# Flags `day launch` takes a SEPARATE value for. Their value is not itself a target, so it has to
# travel with the flag into the passthrough list rather than being classified on its own (the
# `--env DAY_DEMO_ROUTE=canvas` above is the case that makes this visible).
VALUE_FLAGS = {
    "-p", "--platform", "--profile", "--locale", "--env",
    "--script", "--variant", "--project", "--format",
}

GROUPS = {
    "desktop": lambda t: t["kind"] == "desktop",
    "mobile": lambda t: t["kind"] in ("iosSim", "android", "harmonyOs"),
    "web": lambda t: t["kind"] == "web",
}


def _ansi(stream):
    """Whether to colour `stream`. Windows consoles need VT processing turned on explicitly."""
    if os.environ.get("NO_COLOR") is not None or not stream.isatty():
        return False
    if os.name == "nt":
        try:
            import ctypes

            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetStdHandle(-11 if stream is sys.stdout else -12)
            mode = ctypes.c_uint32()
            if not kernel32.GetConsoleMode(handle, ctypes.byref(mode)):
                return False  # redirected, or not a console after all
            # ENABLE_VIRTUAL_TERMINAL_PROCESSING
            kernel32.SetConsoleMode(handle, mode.value | 0x0004)
        except Exception:
            return False
    return True


def step(message):
    # Flushed: cargo and `day` write straight to the inherited handles, so a buffered banner (which
    # is what Python gives you the moment stdout is a pipe) would surface after the output it
    # introduces.
    if _ansi(sys.stdout):
        print("\033[1m▶ %s\033[0m" % message, flush=True)
    else:
        print("> %s" % message, flush=True)


def die(message):
    if _ansi(sys.stderr):
        sys.exit("\033[31merror: %s\033[0m" % message)
    sys.exit("error: %s" % message)


# Resolve this file through any symlinks, so the checkout is found relative to the SCRIPT rather
# than to wherever it was linked from — `ln -s .../day/scripts/launch-showcase.py ~/bin/showcase`
# has to keep working. `Path.resolve()` walks the whole link chain and raises on a loop, which is
# what the shell version hand-rolled a hop counter for.
try:
    SELF = Path(__file__).resolve(strict=True)
except OSError as exc:  # ELOOP, or a link left dangling
    die("could not resolve %s: %s" % (__file__, exc))

ROOT = SELF.parent.parent
SHOWCASE = ROOT / "apps" / "showcase"

# A link left behind after the checkout moved would otherwise fail deep inside cargo with nothing
# pointing back at the real cause.
if not (ROOT / "Cargo.toml").is_file() or not SHOWCASE.is_dir():
    die(
        "%s is not a day checkout (resolved from %s) — the script must live in <day>/scripts/"
        % (ROOT, SELF)
    )


def usage():
    """The header comment IS the help text: print the block after the shebang, up to the first line
    that is not a comment. Deriving the range rather than hardcoding one means editing the header
    can never silently truncate `--help` mid-sentence."""
    with SELF.open(encoding="utf-8") as handle:
        for index, line in enumerate(handle):
            if index == 0:
                continue  # shebang
            if not line.startswith("#"):
                return
            print(line[2:].rstrip() if line.startswith("# ") else line[1:].rstrip())


def parse_args(argv):
    targets, passthrough, dry_run = [], [], False
    pending_value = False
    for arg in argv:
        if pending_value:
            # The value of a flag consumed on the previous iteration — never a target, even though
            # it does not start with `-` (`--env DAY_DEMO_ROUTE=canvas`).
            passthrough.append(arg)
            pending_value = False
        elif arg in ("-h", "--help"):
            usage()
            sys.exit(0)
        elif arg in ("-n", "--dry-run"):
            dry_run = True
        elif arg.startswith("-"):
            passthrough.append(arg)
            # `--flag=value` carries its own value; bare `--flag` takes the next argument.
            pending_value = "=" not in arg and arg in VALUE_FLAGS
        else:
            targets.append(arg)
    if pending_value:
        die("%s expects a value" % passthrough[-1])
    return targets, passthrough, dry_run


def resolve_targets(meta, args):
    """Expand names and groups against the CLI's own target catalog.

    Resolved from `day metadata --json` rather than a list kept here: the catalog carries each
    target's kind and the host OS that can build it, so a target added to day is picked up by the
    groups automatically and a typo is checked against the real catalog.
    """
    host = meta["host"]["os"]
    catalog = meta["targetCatalog"]
    buildable = [t for t in catalog if t["host"] in (host, "any")]
    by_name = {t["name"]: t for t in catalog}

    out, seen = [], set()
    for arg in args:
        if arg in GROUPS:
            names = [t["name"] for t in buildable if GROUPS[arg](t)]
            if not names:
                die("no %s target can be built on %s" % (arg, host))
        elif arg in by_name:
            if by_name[arg]["host"] not in (host, "any"):
                die("%s needs a %s host; this is %s" % (arg, by_name[arg]["host"], host))
            names = [arg]
        else:
            die(
                "unknown target or group: %s\n  groups:  %s\n  targets: %s"
                % (arg, " ".join(GROUPS), " ".join(sorted(by_name)))
            )
        for name in names:
            if name not in seen:
                seen.add(name)
                out.append(name)
    return out


def main():
    target_args, passthrough, dry_run = parse_args(sys.argv[1:])
    if not target_args:
        usage()
        return 2

    # --- the CLI from this checkout -------------------------------------------------------------
    # Even a dry run builds it: the target catalog it prints is the whole point of resolving here.
    step("Building the day CLI")
    if subprocess.run(
        ["cargo", "build", "-p", "day-cli"], cwd=str(ROOT), stdout=subprocess.DEVNULL
    ).returncode:
        die("failed to build day-cli in %s" % ROOT)
    day = ROOT / "target" / "debug" / ("day.exe" if os.name == "nt" else "day")

    # --- expand the arguments into target names -------------------------------------------------
    meta_proc = subprocess.run(
        [str(day), "--project", str(SHOWCASE), "metadata", "--json"],
        stdout=subprocess.PIPE,
    )
    if meta_proc.returncode:
        die("could not read the target catalog")
    targets = resolve_targets(json.loads(meta_proc.stdout), target_args)
    if not targets:
        die("no targets to launch")

    if dry_run:
        print("\n".join(targets))
        return 0

    # --- launch ---------------------------------------------------------------------------------
    argv = [str(day), "--project", str(SHOWCASE), "launch"]
    for target in targets:
        argv += ["-p", target]
    # Release unless the caller asked for something else; passing both would be a clap error.
    if not any(a == "--profile" or a.startswith("--profile=") for a in passthrough):
        argv += ["--profile", "release"]
    argv += passthrough

    step("Launching showcase (release): %s" % " ".join(targets))
    if os.name != "nt":
        os.execv(str(day), argv)  # replace this process, as the shell version does
    # Windows has no exec that REPLACES the caller: os.execv there spawns a new process and exits
    # this one, handing the console back while the app is still running and detaching it from
    # Ctrl-C. Stay alive as a thin wrapper and forward the status instead.
    try:
        return subprocess.run(argv).returncode
    except KeyboardInterrupt:
        # `day launch` installs its own Ctrl-C handling and takes the apps down itself; the console
        # delivers the signal to it too, so this side just exits quietly rather than printing a
        # traceback over its shutdown output.
        return 130


if __name__ == "__main__":
    sys.exit(main())
