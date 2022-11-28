mod oauth2;

use clap::Parser;
use futures::prelude::*;
use oauth2::Oauth2TokenManager;
use reqwest::{Body, Client, Url};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about=None)]
struct Args {
    #[arg(short, long)]
    service_account_json: PathBuf,
    #[arg(short, long)]
    package_name: String,
    #[arg(short, long)]
    bundle: PathBuf,
    #[arg(short, long)]
    version_code: String,
}

const ANDROID_PUBLISHER_SCOPE: &str = "https://www.googleapis.com/auth/androidpublisher";

pub struct ApiClient {
    client: Client,
    package_name: String,
    token_manager: Oauth2TokenManager,
    service_endpoint: Url,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppEdit {
    id: String,
    expiry_time_seconds: String,
}

// #[derive(Serialize, Deserialize, Debug)]
// #[serde(rename_all = "camelCase")]
// pub struct TrackList {
//     kind: String,
//     tracks: Vec<Track>,
// }

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    #[serde(skip_serializing_if = "Option::is_none")]
    track: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    releases: Option<Vec<Release>>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_codes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

impl ApiClient {
    pub fn new(package_name: String, token_manager: Oauth2TokenManager) -> Self {
        Self {
            client: Client::new(),
            package_name,
            token_manager,
            service_endpoint: "https://androidpublisher.googleapis.com".parse().unwrap(),
        }
    }

    fn url(&self, path: impl AsRef<str>) -> Url {
        let mut url = self.service_endpoint.clone();
        url.set_path(path.as_ref());
        url
    }

    async fn create_edit(&self) -> eyre::Result<AppEdit> {
        let res = self
            .client
            .post(self.url(format!(
                "/androidpublisher/v3/applications/{}/edits",
                self.package_name
            )))
            .bearer_auth(self.token_manager.token().await?.access_token)
            .json(&serde_json::Value::Object(Default::default()))
            .send()
            .await?;
        if let Err(err) = res.error_for_status_ref() {
            println!("{}", res.text().await?);
            Err(err.into())
        } else {
            Ok(res.json().await?)
        }
    }

    async fn commit_edit(&self, edit_id: &str) -> eyre::Result<AppEdit> {
        let res = self
            .client
            .post(self.url(format!(
                "/androidpublisher/v3/applications/{}/edits/{}:commit",
                self.package_name, edit_id
            )))
            .bearer_auth(self.token_manager.token().await?.access_token)
            .header("Content-Length", "0")
            .send()
            .await?;
        if let Err(err) = res.error_for_status_ref() {
            println!("{}", res.text().await?);
            Err(err.into())
        } else {
            Ok(res.json().await?)
        }
    }

    async fn upload_bundle(&self, edit_id: &str, bundle: tokio::fs::File) -> eyre::Result<()> {
        let total_size = bundle.metadata().await.unwrap().len();
        let mut reader_stream = tokio_util::io::ReaderStream::new(bundle);
        let mut uploaded = 0;
        let bar = indicatif::ProgressBar::new(total_size);

        let async_stream = async_stream::stream! {
            while let Some(chunk) = reader_stream.next().await {
                if let Ok(chunk) = &chunk {
                    let new = total_size.min(uploaded + (chunk.len() as u64));
                    uploaded = new;
                    bar.set_position(new);
                    if(uploaded >= total_size){
                        bar.finish();
                    }
                }
                yield chunk;
            }
        };

        let res = self
            .client
            .post(self.url(format!(
                "/upload/androidpublisher/v3/applications/{}/edits/{}/bundles",
                self.package_name, edit_id,
            )))
            .bearer_auth(self.token_manager.token().await?.access_token)
            .header("Content-type", "application/octet-stream")
            .body(Body::wrap_stream(async_stream))
            .send()
            .await?;
        if let Err(err) = res.error_for_status_ref() {
            println!("{}", res.text().await?);
            Err(err.into())
        } else {
            Ok(())
        }
    }

    // async fn list_tracks(&self, edit_id: &str) -> eyre::Result<TrackList> {
    //     let res = self
    //         .client
    //         .get(self.url(format!(
    //             "/androidpublisher/v3/applications/{}/edits/{}/tracks",
    //             self.package_name, edit_id
    //         )))
    //         .bearer_auth(self.token_manager.token().await?.access_token)
    //         .send()
    //         .await?;
    //     if let Err(err) = res.error_for_status_ref() {
    //         println!("{}", res.text().await?);
    //         Err(err.into())
    //     } else {
    //         Ok(res.json().await?)
    //     }
    // }

    async fn update_track(&self, edit_id: &str, version_code: String) -> eyre::Result<()> {
        let res = self
            .client
            .put(self.url(format!(
                "/androidpublisher/v3/applications/{}/edits/{}/tracks/internal",
                self.package_name, edit_id
            )))
            .bearer_auth(self.token_manager.token().await?.access_token)
            .json(&Track {
                releases: Some(vec![Release {
                    status: Some("draft".into()),
                    version_codes: Some(vec![version_code]),
                    ..Default::default()
                }]),
                ..Default::default()
            })
            .send()
            .await?;
        if let Err(err) = res.error_for_status_ref() {
            println!("{}", res.text().await?);
            Err(err.into())
        } else {
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    let service_account = serde_json::from_reader(std::fs::File::open(args.service_account_json)?)?;
    let bundle = tokio::fs::File::open(args.bundle).await?;
    let token_manager = Oauth2TokenManager::new(service_account, [ANDROID_PUBLISHER_SCOPE]);
    let client = ApiClient::new(args.package_name, token_manager);

    let edit = client.create_edit().await?;
    client.upload_bundle(&edit.id, bundle).await?;
    client.update_track(&edit.id, args.version_code).await?;
    client.commit_edit(&edit.id).await?;

    Ok(())
}
