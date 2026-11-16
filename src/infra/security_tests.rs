use super::*;
use crate::infra::{instance, lock, paths::Namespace};
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Security::{
    GetSecurityDescriptorControl, SE_DACL_PROTECTED, SetKernelObjectSecurity,
};

#[test]
fn descriptor_is_explicit_protected_owner_only_and_permissive_lock_is_rejected() {
    let user = UserSecurity::current().unwrap();
    let mut control = 0;
    let mut revision = 0;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(user.descriptor.0, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    let namespace = Namespace::isolated();
    let file = lock::open(&namespace, "live").unwrap();
    let sddl = wide(&format!("O:{}D:P(A;;GA;;;WD)", user.sid_text));
    let mut descriptor = ptr::null_mut();
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        },
        0
    );
    let descriptor = LocalAllocation(descriptor);
    use std::os::windows::fs::OpenOptionsExt;
    let permissions = std::fs::OpenOptions::new()
        .access_mode(0x00040000 /* WRITE_DAC */)
        .open(namespace.directory.join("live.lock"))
        .unwrap();
    assert_ne!(
        unsafe {
            SetKernelObjectSecurity(
                permissions.as_raw_handle(),
                DACL_SECURITY_INFORMATION,
                descriptor.0,
            )
        },
        0
    );
    drop(permissions);
    drop(file);
    assert!(instance::gui_is_alive(&namespace).is_err());
    std::fs::remove_dir_all(namespace.directory).unwrap();
}
