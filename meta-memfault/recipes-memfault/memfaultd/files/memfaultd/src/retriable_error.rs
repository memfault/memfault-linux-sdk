//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{ErrReport, Report, Result};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RetriableError {
    #[error("Temporary server error: {status_code}")]
    ServerError { status_code: u16 },
    #[error("Network error ({source})")]
    NetworkError { source: reqwest::Error },
}

pub trait IgnoreNonRetriableError<T> {
    /// Ignore non-retriable errors, turning them into `Ok(None)`.
    /// If the Err holds a RetriableError, it will be returned as-is.
    fn ignore_non_retriable_errors_with<R: FnMut(&Report)>(self, x: R) -> Result<(), ErrReport>;
}

impl<T> IgnoreNonRetriableError<T> for Result<T> {
    fn ignore_non_retriable_errors_with<R: FnMut(&Report)>(
        self,
        mut on_error: R,
    ) -> Result<(), ErrReport> {
        match self {
            Ok(_) => Ok(()),
            Err(e) => {
                if e.downcast_ref::<RetriableError>().is_some() {
                    Err(e)
                } else {
                    on_error(&e);
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use eyre::eyre;
    use rstest::*;

    use super::*;

    #[rstest]
    #[case(Ok(()), false, true)]
    #[case(Err(eyre!("Some error")), true, true)]
    #[case(Err(eyre!(RetriableError::ServerError { status_code: 503 })), false, false)]
    fn test_ignore_non_retriable_errors_with(
        #[case] result: Result<(), Report>,
        #[case] expected_called: bool,
        #[case] expected_ok: bool,
    ) {
        let mut called = false;
        assert_eq!(
            result
                .ignore_non_retriable_errors_with(|_| called = true)
                .is_ok(),
            expected_ok
        );
        assert_eq!(called, expected_called);
    }
}
