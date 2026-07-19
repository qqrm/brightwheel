use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;

/// An error returned by a DDC/CI or related Windows operation.
#[derive(Debug)]
pub struct DdcError {
    message: String,
    source: Option<io::Error>,
}

impl DdcError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn windows(operation: &str) -> Self {
        Self {
            message: operation.to_owned(),
            source: Some(io::Error::last_os_error()),
        }
    }

    pub(crate) fn windows_code(operation: &str, code: u32) -> Self {
        Self {
            message: operation.to_owned(),
            source: Some(io::Error::from_raw_os_error(code as i32)),
        }
    }
}

impl Display for DdcError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(formatter, "{}: {}", self.message, source),
            None => formatter.write_str(&self.message),
        }
    }
}

impl Error for DdcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

/// Result type used by the `BrightWheel` library.
pub type Result<T> = std::result::Result<T, DdcError>;

#[cfg(test)]
mod tests {
    use super::DdcError;
    use std::error::Error;
    use std::io;

    #[test]
    fn message_errors_have_no_source() {
        let error = DdcError::message("display unavailable");

        assert_eq!(error.to_string(), "display unavailable");
        assert!(error.source().is_none());
    }

    #[test]
    fn windows_code_preserves_the_operation_and_source() {
        let error = DdcError::windows_code("operation failed", 2);

        assert!(error.to_string().starts_with("operation failed: "));
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .and_then(io::Error::raw_os_error),
            Some(2)
        );
    }
}
