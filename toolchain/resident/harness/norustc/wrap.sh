#!/usr/bin/env bash
# Counting stand-in for rustc, rustdoc and cargo in a copy of a toolchain: writes one line
# per run (time, tool, pid, ppid, parent's command line, grandparent's pid and command line,
# cwd, arguments) and becomes the real tool, which sits next to it as <tool>.real.
tool=${0##*/}
here=$(cd "${0%/*}" && pwd)
log=${NR_LOG:-${NR_ROOT:-$HOME/no-rustc}/logs/all.log}
{
  pc=$(tr '\0' ' ' < /proc/$PPID/cmdline 2>/dev/null | cut -c1-400 | tr '\t\n' '  ')
  gpid=$(awk '{print $4}' /proc/$PPID/stat 2>/dev/null)
  gpc=$(tr '\0' ' ' < /proc/$gpid/cmdline 2>/dev/null | cut -c1-400 | tr '\t\n' '  ')
  args=$(printf '%s\x1f' "$@" | tr '\t\n' '  ')
  flock 9
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s.%N)" "$tool" "$$" "$PPID" "$pc" "$gpid" "$gpc" "$PWD" "$args" >&9
} 9>>"$log"
if [ "$tool" = cargo ]; then export CARGO="$here/cargo"; fi
exec -a "$here/$tool" "$here/$tool.real" "$@"
