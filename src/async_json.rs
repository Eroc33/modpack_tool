use snafu::{Snafu,ResultExt};
use tokio::io::{AsyncRead,AsyncReadExt,AsyncWrite,AsyncWriteExt};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Io error: {}", source))]
    Io{
        source: std::io::Error,
    },
    #[snafu(display("Json error: {}", source))]
    Json{
        source: serde_json::Error
    },
}

pub async fn read<T,R>(r: &mut R) -> Result<T,Error>
    where T: serde::de::DeserializeOwned,
          R: AsyncRead + Unpin
{
    let mut json = String::new();
    r.read_to_string(&mut json).await.context(Io)?;
    Ok(serde_json::de::from_str(&json).context(Json)?)
}

pub async fn write<T,W>(w: &mut W, t: &T) -> Result<(),Error>
    where T: serde::Serialize,
          W: AsyncWrite + Unpin
{
    let json = serde_json::to_string(t).context(Json)?;
    w.write_all(json.as_bytes()).await.context(Io)?;
    Ok(())
}

pub async fn write_pretty<T,W>(w: &mut W, t: &T) -> Result<(),Error>
    where T: serde::Serialize,
          W: AsyncWrite + Unpin
{
    let json = serde_json::to_string_pretty(t).context(Json)?;
    w.write_all(json.as_bytes()).await.context(Io)?;
    Ok(())
}