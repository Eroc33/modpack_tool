use tokio::io::{AsyncRead,AsyncReadExt,AsyncWrite,AsyncWriteExt};

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Io error: {}", _0)]
    Io(#[cause] std::io::Error),
    #[fail(display = "Json error: {}", _0)]
    Json(#[cause] serde_json::Error),
}

impl From<std::io::Error> for Error{
    fn from(e: std::io::Error) -> Self{
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error{
    fn from(e: serde_json::Error) -> Self{
        Error::Json(e)
    }
}

pub async fn read<T,R>(r: &mut R) -> Result<T,Error>
    where T: serde::de::DeserializeOwned,
          R: AsyncRead + Unpin
{
    let mut json = String::new();
    r.read_to_string(&mut json).await?;
    Ok(serde_json::de::from_str(&json)?)
}

pub async fn write<T,W>(w: &mut W, t: &T) -> Result<(),Error>
    where T: serde::Serialize,
          W: AsyncWrite + Unpin
{
    let json = serde_json::to_string(t)?;
    w.write_all(json.as_bytes()).await?;
    Ok(())
}

pub async fn write_pretty<T,W>(w: &mut W, t: &T) -> Result<(),Error>
    where T: serde::Serialize,
          W: AsyncWrite + Unpin
{
    let json = serde_json::to_string_pretty(t)?;
    w.write_all(json.as_bytes()).await?;
    Ok(())
}