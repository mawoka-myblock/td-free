use crate::NvsMutex;
use serde::{Serialize, de::DeserializeOwned};
use tickv::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum NvsOpError {
    #[error("TickV error: {0:?}")]
    TickvError(ErrorCode),

    #[error("Postcard error: {0}")]
    PostcardError(#[from] postcard::Error),
}

impl From<ErrorCode> for NvsOpError {
    fn from(err: ErrorCode) -> Self {
        Self::TickvError(err)
    }
}

pub trait NvsStored: Serialize + DeserializeOwned + Sized {
    const KEY: &'static [u8];

    #[allow(async_fn_in_trait)]
    async fn save(&self, nvs_mutex: &'static NvsMutex) -> Result<(), NvsOpError> {
        let nvs = nvs_mutex.lock().await;

        nvs.invalidate_key(Self::KEY).await.or_else(|e| match e {
            ErrorCode::KeyNotFound => Ok(()),
            other => Err(other),
        })?;

        let mut buf = [0u8; 512];
        let data = postcard::to_slice(self, &mut buf)?;

        nvs.append_key(Self::KEY, data).await?;

        Ok(())
    }
    #[allow(async_fn_in_trait)]
    async fn read(nvs_mutex: &'static NvsMutex) -> Result<Option<Self>, NvsOpError> {
        let nvs = nvs_mutex.lock().await;

        let data = match nvs.get_key(Self::KEY).await {
            Ok(data) => data,
            Err(ErrorCode::KeyNotFound) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        Ok(Some(postcard::from_bytes(&data)?))
    }
    #[allow(async_fn_in_trait)]
    async fn delete(nvs_mutex: &'static NvsMutex) -> Result<(), NvsOpError> {
        let nvs = nvs_mutex.lock().await;
        nvs.invalidate_key(Self::KEY).await?;
        Ok(())
    }
}
