use mesh_core::request::{normalize_request, HostsInput};
#[test]
fn wildcard_local_and_dedup_keep_existing_wire_contract() {
 let n=normalize_request("A",vec!["B".into(),"C".into()],None,Some("id".into()),Some("origin".into()),None).unwrap();
 assert_eq!(n.hosts,vec!["A","B","C"]);assert_eq!(n.request_id,"id");assert_eq!(n.origin_host,"origin");
 let n=normalize_request("A",vec!["B".into()],Some(HostsInput::Many(vec!["local".into(),"B".into(),"B".into()])),Some("same".into()),None,None).unwrap();
 assert_eq!(n.hosts,vec!["A","B"]);
}
#[test]
fn forwarded_peer_cannot_fan_out_or_form_a_loop() {
 assert!(normalize_request("B",vec!["C".into()],Some(HostsInput::One("*".into())),None,None,Some(1)).is_err());
 assert!(normalize_request("B",vec![],Some(HostsInput::One("local".into())),None,None,Some(2)).is_err());
 let n=normalize_request("B",vec![],Some(HostsInput::One("local".into())),Some("original".into()),Some("A".into()),Some(1)).unwrap();
 assert_eq!(n.hosts,vec!["B"]);assert_eq!(n.request_id,"original");assert_eq!(n.origin_host,"A");assert_eq!(n.hop_count,1);
}
#[test]
fn unknown_host_is_not_silently_retargeted() {
 assert!(normalize_request("A",vec!["B".into()],Some(HostsInput::One("unknown".into())),None,None,None).is_err());
}
