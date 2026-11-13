//! macOS keychain items that name the Pyoxide binaries in their access list.
//!
//! An item is trusted by the code that created it and by the applications in
//! its access list; anything else makes the system ask for the login
//! password. The `keyring` crate creates items with the default list (the
//! creating binary only), so an item the CLI made would prompt in the IDE and
//! the other way round. The item is created here with an explicit list
//! instead (`SecAccessCreate` + `SecKeychainItemCreateFromContent`, the
//! legacy file-keychain API, the only one with per-application lists).
//!
//! The list holds code requirements, not paths: a binary signed ad hoc is a
//! new identity with every build and stops matching (see
//! `docs/code-signing.md`).

use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::ptr;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework_sys::base::{
    SecAccessRef, SecKeychainAttribute, SecKeychainAttributeList, SecKeychainItemRef,
};

type OSStatus = i32;
type SecTrustedApplicationRef = *mut c_void;

const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;

/// `kSecGenericPasswordItemClass` ('genp').
const GENERIC_PASSWORD_CLASS: u32 = 0x6765_6E70;
/// `kSecServiceItemAttr` ('svce').
const SERVICE_ATTR: u32 = 0x7376_6365;
/// `kSecAccountItemAttr` ('acct').
const ACCOUNT_ATTR: u32 = 0x6163_6374;

/// Binaries that read and write data source passwords and ship together
/// (the product's programs, `app_info::binaries`).
fn trusted_names() -> &'static [&'static str] {
    app_info::binaries()
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustedApplicationCreateFromPath(
        path: *const c_char,
        app: *mut SecTrustedApplicationRef,
    ) -> OSStatus;
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: CFTypeRef,
        access: *mut SecAccessRef,
    ) -> OSStatus;
    fn SecKeychainItemCreateFromContent(
        item_class: u32,
        attr_list: *mut SecKeychainAttributeList,
        length: u32,
        data: *const c_void,
        keychain: CFTypeRef,
        initial_access: SecAccessRef,
        item: *mut SecKeychainItemRef,
    ) -> OSStatus;
    fn SecKeychainFindGenericPassword(
        keychain_or_array: CFTypeRef,
        service_len: u32,
        service: *const c_char,
        account_len: u32,
        account: *const c_char,
        password_len: *mut u32,
        password: *mut *mut c_void,
        item: *mut SecKeychainItemRef,
    ) -> OSStatus;
    static kSecACLAuthorizationDecrypt: CFStringRef;
    #[cfg(test)]
    static kSecACLAuthorizationChangeACL: CFStringRef;
    fn SecKeychainItemCopyAccess(item: SecKeychainItemRef, access: *mut SecAccessRef) -> OSStatus;
    fn SecAccessCopyMatchingACLList(access: SecAccessRef, authorization: CFTypeRef) -> CFArrayRef;
    fn SecACLCopyContents(
        acl: CFTypeRef,
        applications: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt_selector: *mut u16,
    ) -> OSStatus;
    fn SecACLSetContents(
        acl: CFTypeRef,
        applications: CFArrayRef,
        description: CFStringRef,
        prompt_selector: u16,
    ) -> OSStatus;
    fn SecKeychainItemSetAccess(item: SecKeychainItemRef, access: SecAccessRef) -> OSStatus;
    fn SecKeychainItemModifyAttributesAndData(
        item: SecKeychainItemRef,
        attrs: *const SecKeychainAttributeList,
        length: u32,
        data: *const c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

fn status(code: OSStatus, what: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{what} failed (OSStatus {code})"))
    }
}

/// The running binary and its siblings that share the credential store.
pub(crate) fn trusted_binaries() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let mut list = vec![exe.clone()];
    if let Some(dir) = exe.parent() {
        for name in trusted_names() {
            let path = dir.join(name);
            if path.is_file() && !list.contains(&path) {
                list.push(path);
            }
        }
    }
    list
}

/// Trusted applications for existing binaries; missing siblings are skipped.
fn trusted_apps(binaries: &[PathBuf]) -> Result<CFArray<CFType>, String> {
    let mut apps: Vec<CFType> = Vec::new();
    for path in binaries.iter().filter(|p| p.is_file()) {
        let c_path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "binary path contains NUL".to_owned())?;
        let mut app: SecTrustedApplicationRef = ptr::null_mut();
        // SAFETY: valid NUL-terminated path and out pointer.
        status(
            unsafe { SecTrustedApplicationCreateFromPath(c_path.as_ptr(), &mut app) },
            "SecTrustedApplicationCreateFromPath",
        )?;
        // SAFETY: the call returned a +1 reference owned by us.
        apps.push(unsafe { CFType::wrap_under_create_rule(app as CFTypeRef) });
    }
    Ok(CFArray::from_CFTypes(&apps))
}

fn access_for(descriptor: &str, binaries: &[PathBuf]) -> Result<SecAccessRef, String> {
    let list = trusted_apps(binaries)?;
    let descriptor = CFString::new(descriptor);
    let mut access: SecAccessRef = ptr::null_mut();
    // SAFETY: valid CF objects; `access` is a +1 reference returned to us.
    status(
        unsafe {
            SecAccessCreate(
                descriptor.as_concrete_TypeRef(),
                list.as_concrete_TypeRef() as CFTypeRef,
                &mut access,
            )
        },
        "SecAccessCreate",
    )?;
    Ok(access)
}

/// Changes the matching ACL entries in memory, retaining their prompt policy
/// and identity. A fresh SecAccess would add entries rather than replace them.
fn replace_applications(
    access: SecAccessRef,
    authorization: CFTypeRef,
    binaries: &[PathBuf],
) -> Result<(), String> {
    let apps = trusted_apps(binaries)?;
    // SAFETY: access and the authorization constant are valid CF references.
    let acls = unsafe { SecAccessCopyMatchingACLList(access, authorization) };
    if acls.is_null() {
        return Err("Keychain access has no matching application ACL".into());
    }
    // SAFETY: Copy returned a +1 array of ACL references.
    let acls = unsafe { CFArray::<CFType>::wrap_under_create_rule(acls) };
    if acls.is_empty() {
        return Err("Keychain access has no matching application ACL".into());
    }
    for acl in acls.iter() {
        let mut old_apps = ptr::null();
        let mut description = ptr::null();
        let mut prompt_selector = 0;
        // SAFETY: the ACL is retained by the array, all outputs are valid.
        status(
            unsafe {
                SecACLCopyContents(
                    acl.as_CFTypeRef(),
                    &mut old_apps,
                    &mut description,
                    &mut prompt_selector,
                )
            },
            "SecACLCopyContents",
        )?;
        // SAFETY: copied references are owned; NULL application lists are allowed.
        let result = unsafe {
            SecACLSetContents(
                acl.as_CFTypeRef(),
                apps.as_concrete_TypeRef(),
                description,
                prompt_selector,
            )
        };
        unsafe {
            if !old_apps.is_null() {
                CFRelease(old_apps.cast());
            }
            if !description.is_null() {
                CFRelease(description.cast());
            }
        }
        status(result, "SecACLSetContents")?;
    }
    Ok(())
}

fn keychain_ref(keychain: Option<&SecKeychain>) -> CFTypeRef {
    keychain.map_or(ptr::null(), |k| k.as_concrete_TypeRef() as CFTypeRef)
}

/// Finds an item without reading its secret. The returned reference is owned.
fn find_item(
    keychain: Option<&SecKeychain>,
    service: &str,
    account: &str,
) -> Result<Option<CFType>, String> {
    let mut item: SecKeychainItemRef = ptr::null_mut();
    // SAFETY: lengths match the buffers; no password out parameters.
    let found = unsafe {
        SecKeychainFindGenericPassword(
            keychain_ref(keychain),
            service.len() as u32,
            service.as_ptr().cast(),
            account.len() as u32,
            account.as_ptr().cast(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut item,
        )
    };
    if found == ERR_SEC_ITEM_NOT_FOUND {
        return Ok(None);
    }
    status(found, "SecKeychainFindGenericPassword")?;
    // SAFETY: the successful lookup returned a +1 reference owned by us.
    Ok(Some(unsafe {
        CFType::wrap_under_create_rule(item as CFTypeRef)
    }))
}

/// Stores the secret in an item whose access list names `binaries`,
/// replacing an existing item (its list may predate them). `keychain: None`
/// is the user's default keychain.
pub(crate) fn set_trusted(
    keychain: Option<&SecKeychain>,
    service: &str,
    account: &str,
    secret: &[u8],
    binaries: &[PathBuf],
) -> Result<(), String> {
    if let Some(item) = find_item(keychain, service, account)? {
        let item = item.as_CFTypeRef() as SecKeychainItemRef;
        let mut access = ptr::null_mut();
        // SAFETY: item is live and the output receives an owned reference.
        status(
            unsafe { SecKeychainItemCopyAccess(item, &mut access) },
            "SecKeychainItemCopyAccess",
        )?;
        // SAFETY: Copy returned a +1 reference.
        let access_owner = unsafe { CFType::wrap_under_create_rule(access.cast()) };
        replace_applications(
            access,
            unsafe { kSecACLAuthorizationDecrypt.cast() },
            binaries,
        )?;
        // Update access first: denied ACL changes must not replace the password.
        // If the data write fails, the old password remains; its application
        // list may already have been refreshed. Never delete or retry as creation.
        // SAFETY: both references are live for the call.
        status(
            unsafe { SecKeychainItemSetAccess(item, access) },
            "SecKeychainItemSetAccess",
        )?;
        drop(access_owner);
        // SAFETY: item is live and the secret buffer outlives the call.
        return status(
            unsafe {
                SecKeychainItemModifyAttributesAndData(
                    item,
                    ptr::null(),
                    secret.len() as u32,
                    secret.as_ptr().cast(),
                )
            },
            "SecKeychainItemModifyAttributesAndData",
        );
    }
    let access = access_for(service, binaries)?;
    let result = create_item(keychain, service, account, secret, access);
    // SAFETY: access_for returned a +1 reference.
    unsafe { CFRelease(access.cast()) };
    result
}

fn create_item(
    keychain: Option<&SecKeychain>,
    service: &str,
    account: &str,
    secret: &[u8],
    access: SecAccessRef,
) -> Result<(), String> {
    let mut attrs = [
        SecKeychainAttribute {
            tag: SERVICE_ATTR,
            length: service.len() as u32,
            data: service.as_ptr().cast_mut().cast(),
        },
        SecKeychainAttribute {
            tag: ACCOUNT_ATTR,
            length: account.len() as u32,
            data: account.as_ptr().cast_mut().cast(),
        },
    ];
    let mut list = SecKeychainAttributeList {
        count: attrs.len() as u32,
        attr: attrs.as_mut_ptr(),
    };
    let mut item: SecKeychainItemRef = ptr::null_mut();
    // SAFETY: the attribute buffers outlive the call (the call copies).
    let created = unsafe {
        SecKeychainItemCreateFromContent(
            GENERIC_PASSWORD_CLASS,
            &mut list,
            secret.len() as u32,
            secret.as_ptr().cast(),
            keychain_ref(keychain),
            access,
            &mut item,
        )
    };
    if !item.is_null() {
        // SAFETY: +1 reference we do not keep.
        unsafe { CFRelease(item as CFTypeRef) };
    }
    status(created, "SecKeychainItemCreateFromContent")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use security_framework::os::macos::keychain::CreateOptions;

    use super::*;

    /// A scratch keychain file that is not in the search list; the person's
    /// keychains are never touched. Optional prompts are disabled; tests that
    /// change ACLs also authorize the test executable in the scratch owner ACL.
    struct Scratch {
        _dir: tempfile::TempDir,
        path: PathBuf,
        keychain: SecKeychain,
    }

    fn scratch() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pyoxide-test.keychain-db");
        let keychain = CreateOptions::new().password("x").create(&path).unwrap();
        Scratch {
            _dir: dir,
            path,
            keychain,
        }
    }

    impl Scratch {
        fn acl_dump(&self) -> String {
            let out = Command::new("security")
                .arg("dump-keychain")
                .arg("-a")
                .arg(&self.path)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    }

    #[test]
    fn item_trusts_the_listed_binaries_and_replaces_older_lists() {
        let _no_prompts = SecKeychain::disable_user_interaction().unwrap();
        let k = scratch();
        let exe = std::env::current_exe().unwrap();
        let ide = PathBuf::from("/usr/bin/true");
        let cli = PathBuf::from("/usr/bin/false");

        let access = access_for("pyoxide-test", &[exe.clone(), ide.clone(), cli.clone()]).unwrap();
        // Only the scratch fixture trusts this test executable to change ACLs.
        // Default owner ACLs require a password dialog even when optional
        // keychain interaction has been disabled.
        replace_applications(
            access,
            unsafe { kSecACLAuthorizationChangeACL.cast() },
            std::slice::from_ref(&exe),
        )
        .unwrap();
        create_item(
            Some(&k.keychain),
            "pyoxide-test",
            "acct",
            b"secret-1",
            access,
        )
        .unwrap();
        unsafe { CFRelease(access.cast()) };
        let dump = k.acl_dump();
        for path in [&ide, &cli] {
            assert!(
                dump.contains(&path.display().to_string()),
                "access list lacks {}:\n{dump}",
                path.display()
            );
        }
        let (password, _) = k
            .keychain
            .find_generic_password("pyoxide-test", "acct")
            .unwrap();
        assert_eq!(&*password, b"secret-1");
        drop(password);

        // Setting again replaces the value and the list.
        set_trusted(
            Some(&k.keychain),
            "pyoxide-test",
            "acct",
            b"secret-2",
            &[exe, ide.clone()],
        )
        .unwrap();
        let dump = k.acl_dump();
        assert!(dump.contains(&ide.display().to_string()));
        assert!(!dump.contains(&cli.display().to_string()), "{dump}");
        let (password, _) = k
            .keychain
            .find_generic_password("pyoxide-test", "acct")
            .unwrap();
        assert_eq!(&*password, b"secret-2");
        assert_eq!(
            dump.matches("class: \"genp\"").count(),
            1,
            "one item\n{dump}"
        );
    }

    #[test]
    fn locked_keychain_update_preserves_the_original_password() {
        let _no_prompts = SecKeychain::disable_user_interaction().unwrap();
        let mut k = scratch();
        let binaries = [std::env::current_exe().unwrap()];
        set_trusted(
            Some(&k.keychain),
            "pyoxide-test",
            "acct",
            b"original",
            &binaries,
        )
        .unwrap();
        let locked = Command::new("security")
            .arg("lock-keychain")
            .arg(&k.path)
            .status()
            .unwrap();
        assert!(locked.success());

        let result = set_trusted(
            Some(&k.keychain),
            "pyoxide-test",
            "acct",
            b"replacement",
            &binaries,
        );
        assert!(result.is_err(), "locked update unexpectedly succeeded");
        k.keychain.unlock(Some("x")).unwrap();
        let (password, _) = k
            .keychain
            .find_generic_password("pyoxide-test", "acct")
            .unwrap();
        assert_eq!(&*password, b"original");
        assert_eq!(k.acl_dump().matches("class: \"genp\"").count(), 1);
    }

    #[test]
    fn trusted_binaries_start_with_the_running_one() {
        let list = trusted_binaries();
        assert_eq!(list[0], std::env::current_exe().unwrap());
    }
}
