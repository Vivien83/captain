use super::*;

impl RunStore {
    pub(super) fn seal_json(&self, context: &str, value: &Value) -> Result<Vec<u8>, ExecutorError> {
        self.cipher.seal(context, &serde_json::to_vec(value)?)
    }

    pub(super) fn open_json(&self, context: &str, blob: &[u8]) -> Result<Value, ExecutorError> {
        Ok(serde_json::from_slice(
            &self.cipher.open_blob(context, blob)?,
        )?)
    }

    pub(super) fn seal_text(&self, context: &str, value: &str) -> Result<Vec<u8>, ExecutorError> {
        self.cipher.seal(context, value.as_bytes())
    }

    pub(super) fn open_optional_json(
        &self,
        context: &str,
        blob: Option<Vec<u8>>,
    ) -> Result<Option<Value>, ExecutorError> {
        blob.map(|blob| self.open_json(context, &blob)).transpose()
    }

    pub(super) fn open_optional_text(
        &self,
        context: &str,
        blob: Option<Vec<u8>>,
    ) -> Result<Option<String>, ExecutorError> {
        blob.map(|blob| {
            String::from_utf8(self.cipher.open_blob(context, &blob)?.to_vec())
                .map_err(|error| ExecutorError::InvalidState(error.to_string()))
        })
        .transpose()
    }
}
