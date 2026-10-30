// bun's Windows calls on Linux (`GetLastError`) and C++ bindings of classes it leaves out
// (`ShellInterpreter__getConstructor`): code nothing reaches calls what nothing defines, and
// the link takes the code out.
unsafe extern "C" {
    fn ra_jit_defined_nowhere() -> u32;
}

pub fn windows_only() -> u32 {
    unsafe { ra_jit_defined_nowhere() }
}

pub fn greeting() -> &'static str {
    "linked without what nothing reaches"
}
