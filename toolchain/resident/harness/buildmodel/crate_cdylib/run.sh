set -e
case "$(rustc -vV)" in
*msvc*)
    # MSVC: the library is linked by what names its exports, and found next to the program.
    cl -nologo -Fe"$ARTIFACT.user.exe" -Fo"$ARTIFACT.user.obj" user.c -link $RA_JIT_OBJECTS "$ARTIFACT.lib" -FORCE:MULTIPLE >"$ARTIFACT.user.log" 2>&1 || { cat "$ARTIFACT.user.log" >&2; exit 1; }
    "$ARTIFACT.user.exe"
    exit
    ;;
esac
cc -o "$ARTIFACT.user" user.c $RA_JIT_OBJECTS "$ARTIFACT" -Wl,--allow-multiple-definition -lpthread -ldl -lm
LD_LIBRARY_PATH="$(dirname "$ARTIFACT"):$LD_LIBRARY_PATH" "$ARTIFACT.user"
