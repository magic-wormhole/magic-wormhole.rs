#![allow(dead_code)]

use super::*;

#[allow(unused_variables, unused_mut)]
pub async fn send_file<F, N, H>(
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    file: &mut F,
    file_path: N,
    file_name: String,
    progress_handler: H,
    peer_version: AppVersion,
) -> Result<(), TransferError>
where
    F: AsyncRead + Unpin,
    N: Into<PathBuf>,
    H: FnMut(u64, u64) + 'static,
{
    // let file_path = file_path.into();
    // let peer_abilities = peer_version.transfer_v2.unwrap();
    // let mut actual_transit_abilities = transit::Ability::all_abilities();
    // actual_transit_abilities.retain(|a| peer_abilities.transit_abilities.contains(a));
    // let connector = transit::init(actual_transit_abilities, Some(&peer_abilities.transit_abilities), relay_hints).await?;

    // /* Send our transit hints */
    // wormhole
    // .send_json(
    //     &PeerMessage::transit_v2(
    //         (**connector.our_hints()).clone().into(),
    //     ),
    // )
    // .await?;

    // /* Receive their transit hints */
    // let their_hints: transit::Hints =
    //     match wormhole.receive_json().await?? {
    //         PeerMessage::TransitV2(transit) => {
    //             debug!("received transit message: {:?}", transit);
    //             transit.hints_v2.into()
    //         },
    //         PeerMessage::Error(err) => {
    //             bail!(TransferError::PeerError(err));
    //         },
    //         other => {
    //             let error = TransferError::unexpected_message("transit-v2", other);
    //             let _ = wormhole
    //                 .send_json(&PeerMessage::Error(format!("{}", error)))
    //                 .await;
    //             bail!(error)
    //         },
    //     };

    // /* Get a transit connection */
    // let mut transit = match connector
    //     .leader_connect(
    //         wormhole.key().derive_transit_key(wormhole.appid()),
    //         Arc::new(peer_abilities.transit_abilities),
    //         Arc::new(their_hints),
    //     )
    //     .await
    // {
    //     Ok(transit) => transit,
    //     Err(error) => {
    //         let error = TransferError::TransitConnect(error);
    //         let _ = wormhole
    //             .send_json(&PeerMessage::Error(format!("{}", error)))
    //             .await;
    //         return Err(error);
    //     },
    // };

    // /* Close the Wormhole and switch to using the transit connection (msgpack instead of json) */
    // wormhole.close().await?;

    // transit.send_record(&PeerMessage::OfferV2(OfferV2 {
    //     transfer_name: None,
    //     files: vec![],
    //     format: "tar.zst".into(),
    // }).ser_msgpack()).await?;

    // match PeerMessage::de_msgpack(&transit.receive_record().await?)? {
    //     PeerMessage::AnswerV2(answer) => {
    //         // let files = answer.files;
    //     },
    //     PeerMessage::Error(err) => {
    //         bail!(TransferError::PeerError(err));
    //     },
    //     other => {
    //         let error = TransferError::unexpected_message("answer-v2", other);
    //         let _ = transit
    //             .send_record(&PeerMessage::Error(format!("{}", error)).ser_msgpack())
    //             .await;
    //         bail!(error)
    //     },
    // }

    Ok(())
}

#[allow(unused_variables)]
pub async fn send_folder<N, M, H>(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    folder_path: N,
    folder_name: M,
    progress_handler: H,
) -> Result<(), TransferError>
where
    N: Into<PathBuf>,
    M: Into<PathBuf>,
    H: FnMut(u64, u64) + 'static,
{
    unimplemented!()
}
