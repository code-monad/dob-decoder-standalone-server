use std::fs;
use std::path::PathBuf;

use jsonrpsee::core::async_trait;
use jsonrpsee::{proc_macros::rpc, tracing, types::ErrorCode};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};

use crate::decoder::DOBDecoder;
use crate::types::Error;

// decoding result contains rendered result from native decoder and DNA string for optional use
#[derive(Serialize, Clone, Debug)]
pub struct ServerDecodeResult {
    render_output: Value,
    dob_content: Value,
}

#[rpc(server)]
trait DecoderRpc {
    #[method(name = "dob_protocol_version")]
    async fn protocol_versions(&self) -> Vec<String>;

    #[method(name = "dob_decode")]
    async fn decode(&self, hexed_spore_id: String) -> Result<Value, ErrorCode>;

    #[method(name = "dob_batch_decode")]
    async fn batch_decode(&self, hexed_spore_ids: Vec<String>) -> Result<Vec<Value>, ErrorCode>;
}

pub struct DecoderStandaloneServer {
    decoder: DOBDecoder,
}

impl DecoderStandaloneServer {
    pub fn new(decoder: DOBDecoder) -> Self {
        Self { decoder }
    }
}

#[async_trait]
impl DecoderRpcServer for DecoderStandaloneServer {
    async fn protocol_versions(&self) -> Vec<String> {
        self.decoder.protocol_versions()
    }

    // decode DNA in particular spore DOB cell
    async fn decode(&self, hexed_spore_id: String) -> Result<Value, ErrorCode>  {
        let decoded_data = decode_dob(&self.decoder, hexed_spore_id).await;
        match decoded_data {
            Ok(result) => Ok(json!(result)),
            Err(error) => Err(error.into()),
        }
    }

    // decode DNA from a set
    async fn batch_decode(&self, hexed_spore_ids: Vec<String>) -> Result<Vec<Value>, ErrorCode> {
        let mut await_results = Vec::new();
        for hexed_spore_id in hexed_spore_ids {
            await_results.push(self.decode(hexed_spore_id));
        }
        let results = futures::future::join_all(await_results)
            .await
            .into_iter()
            .map(|result| match result {
                Ok(result) => Ok(result),
                Err(error) => Err(error),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }
}


pub async fn decode_dob(decoder: &DOBDecoder,hexed_spore_id: String) -> Result<ServerDecodeResult, ErrorCode> {
    tracing::info!("decoding spore_id {hexed_spore_id}");
        let spore_id: [u8; 32] = hex::decode(hexed_spore_id.clone())
            .map_err(|_| Error::HexedSporeIdParseError)?
            .try_into()
            .map_err(|_| Error::SporeIdLengthInvalid)?;
        let mut cache_path = decoder.setting().dobs_cache_directory.clone();
        cache_path.push(format!("{}.dob", hex::encode(spore_id)));
        let (render_output, dob_content) = if cache_path.exists() {
            read_dob_from_cache(cache_path)?
        } else {
            let ((content, dna), metadata) =
                decoder.fetch_decode_ingredients(spore_id).await?;
            let render_output = decoder.decode_dna(&dna, metadata).await?;
            write_dob_to_cache(&render_output, &content, cache_path)?;
            (render_output, content)
        };
        let result = ServerDecodeResult {
            render_output: serde_json::from_str(render_output.as_str()).unwrap(),
            dob_content,
        };
        tracing::info!("spore_id {hexed_spore_id}, result: {}", result.render_output);
        Ok(result)
}

pub fn read_dob_from_cache(cache_path: PathBuf) -> Result<(String, Value), Error> {
    let file_content = fs::read_to_string(cache_path).map_err(|_| Error::DOBRenderCacheNotFound)?;
    let mut lines = file_content.split('\n');
    let (Some(result), Some(content)) = (lines.next(), lines.next()) else {
        return Err(Error::DOBRenderCacheModified);
    };
    match serde_json::from_str(content) {
        Ok(content) => Ok((result.to_string(), content)),
        Err(_) => Err(Error::DOBRenderCacheModified),
    }
}

pub fn write_dob_to_cache(
    render_result: &str,
    dob_content: &Value,
    cache_path: PathBuf,
) -> Result<(), Error> {
    let json_dob_content = serde_json::to_string(dob_content).unwrap();
    let file_content = format!("{render_result}\n{json_dob_content}");
    fs::write(cache_path, file_content).map_err(|_| Error::DOBRenderCacheNotFound)?;
    Ok(())
}
