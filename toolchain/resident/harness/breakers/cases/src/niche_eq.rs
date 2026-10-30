//! fd: `ExitCode::GeneralError.exit()` left with 130, the code of `KilledBySigint`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success,
    HasResults(bool),
    GeneralError,
    KilledBySigint,
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        match code {
            ExitCode::Success => 0,
            ExitCode::HasResults(has_results) => !has_results as i32,
            ExitCode::GeneralError => 1,
            ExitCode::KilledBySigint => 130,
        }
    }
}

impl ExitCode {
    #[inline(never)]
    fn exit(self) -> (bool, i32) {
        (self == ExitCode::KilledBySigint, self.into())
    }
}

pub fn run() -> Vec<(bool, i32)> {
    [ExitCode::Success, ExitCode::HasResults(true), ExitCode::HasResults(false), ExitCode::GeneralError, ExitCode::KilledBySigint]
        .map(ExitCode::exit)
        .to_vec()
}
