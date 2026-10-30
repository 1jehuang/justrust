set -e
case "$(rustc -vV)" in
*msvc*)
    # MSVC: the libraries of the system that std names, as `--print native-static-libs` lists them
    # (nightly-2026-10-25: `synchronization.lib` too, for WaitOnAddress).
    cl -nologo -Fe"$ARTIFACT.user.exe" -Fo"$ARTIFACT.user.obj" user.c -link $RA_JIT_OBJECTS "$ARTIFACT" kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib synchronization.lib -FORCE:MULTIPLE >"$ARTIFACT.user.log" 2>&1 || { cat "$ARTIFACT.user.log" >&2; exit 1; }
    "$ARTIFACT.user.exe"
    exit
    ;;
esac
cc -o "$ARTIFACT.user" user.c $RA_JIT_OBJECTS "$ARTIFACT" -Wl,--allow-multiple-definition -lpthread -ldl -lm
LD_LIBRARY_PATH="$(dirname "$ARTIFACT"):$LD_LIBRARY_PATH" "$ARTIFACT.user"
