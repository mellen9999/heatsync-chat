// the synced settings catalog the ext's registry writes, as the sync table is held against it. the file is the
// site's shared/settings-sync.vectors.json, vendored byte for byte by scripts/sync-emote-vectors.sh
use super::*;

const CATALOG: &str = include_str!("../settings-sync.vectors.json");

fn catalog() -> Value {
  serde_json::from_str(CATALOG).unwrap()
}

// the ext's keys, by storage key: type, scope, default, options
pub fn fixture() -> Value {
  catalog()["synced"].clone()
}

fn names_in(list: &str) -> Vec<String> {
  catalog()[list].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
}

#[test]
fn the_vendored_catalog_is_the_one_synced() {
  let sum = ring::digest::digest(&ring::digest::SHA256, CATALOG.as_bytes());
  let hex: String = sum.as_ref().iter().map(|b| format!("{b:02x}")).collect();
  assert_eq!(hex, include_str!("../settings-sync.vectors.sha256").trim(), "run scripts/sync-emote-vectors.sh");
}

// no key we read or write is one the ext keeps on one device, retired, or that the server refuses
#[test]
fn no_mapped_key_is_one_the_account_does_not_carry() {
  let (local, retired, refused) = (names_in("local"), names_in("retired"), names_in("deviceLocal"));
  let mut keys: Vec<&str> = mapped().filter_map(|d| d.wire.key()).collect();
  keys.extend(TEXTS.iter().map(|(k, _)| *k));
  for k in keys {
    assert!(!local.iter().any(|l| l == k), "{k}: the ext keeps it on one device");
    assert!(!retired.iter().any(|r| r == k), "{k}: the ext retired it");
    assert!(!refused.iter().any(|r| r == k), "{k}: the server refuses it");
  }
  // the keys the server refuses: it rejects the WHOLE ui-state:sync patch if one is in it (ws-handlers
  // UI_STATE_DEVICE_LOCAL_KEYS), so no patch of ours may hold one
  assert_eq!(refused, ["platformFilters"]);
  // the rules ride the account up to the ext's own cap
  assert_eq!(catalog()["largeKeyMax"].as_u64(), text_cap(RULES).map(|c| c as u64));
  assert_eq!(names_in("largeKeys"), [RULES]);
}

// the keys the server refuses in a patch, from the catalog
pub fn device_local() -> Vec<String> {
  names_in("deviceLocal")
}
