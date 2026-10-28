//! Passwords and SSH secrets, kept only in the macOS Keychain or the Windows Credential Manager.

use keyring_core::Entry;
use tu_core::{Error, ErrorKind};

const SERVICE: &str = "com.notinferred.tableultra";

pub fn init() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new().map_err(keychain)?);
    #[cfg(target_os = "windows")]
    keyring_core::set_default_store(windows_native_keyring_store::Store::new().map_err(keychain)?);
    Ok(())
}

pub fn password_account(connection_id: &str) -> String {
    connection_id.to_string()
}

pub fn ssh_account(connection_id: &str) -> String {
    format!("{connection_id}/ssh")
}

pub fn get(account: &str) -> Result<Option<String>, Error> {
    match Entry::new(SERVICE, account).and_then(|e| e.get_password()) {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain(e)),
    }
}

pub fn set(account: &str, secret: &str) -> Result<(), Error> {
    Entry::new(SERVICE, account).and_then(|e| e.set_password(secret)).map_err(keychain)
}

pub fn delete(account: &str) -> Result<(), Error> {
    match Entry::new(SERVICE, account).and_then(|e| e.delete_credential()) {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(keychain(e)),
    }
}

fn keychain(e: keyring_core::Error) -> Error {
    Error::new(ErrorKind::Keychain, format!("The system keychain refused: {e}."))
}
