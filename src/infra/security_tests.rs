use super::*;
use crate::infra::{instance, ipc::PipeClient};
use std::ptr;
use windows_sys::Win32::{
    Foundation::ERROR_ACCESS_DENIED,
    Security::{Authorization::SE_KERNEL_OBJECT, GetSecurityDescriptorControl, SE_DACL_PROTECTED},
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    },
    System::{
        Pipes::{CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS},
        Threading::CreateMutexW,
    },
};

fn descriptor(sddl: &str) -> LocalAllocation {
    let mut sd = ptr::null_mut();
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(sddl).as_ptr(),
                1,
                &mut sd,
                ptr::null_mut(),
            )
        },
        0
    );
    LocalAllocation(sd)
}
fn attributes(sd: &LocalAllocation) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    }
}

#[test]
fn real_objects_have_explicit_protected_current_user_owner_and_only_one_user_ace() {
    let user = UserSecurity::current().unwrap();
    let mut control = 0;
    let mut revision = 0;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(user.descriptor.0, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    let namespace = Namespace::isolated();
    let handle = Handle::new(unsafe {
        CreateMutexW(&user.attributes(), 0, namespace.mutex("acl").as_ptr())
    })
    .unwrap();
    user.verify(handle.0, SE_KERNEL_OBJECT).unwrap();
    let production_a = Namespace::current_user().unwrap();
    let production_b = Namespace::current_user().unwrap();
    // Names alone are examined: production objects are never created/opened.
    assert_eq!(production_a.stem, production_b.stem);
    assert!(production_a.stem.contains(&user.sid_text));
}

#[test]
fn precreated_permissive_mutex_is_rejected_instead_of_trusted_as_absent_gui() {
    let user = UserSecurity::current().unwrap();
    let namespace = Namespace::isolated();
    let sd = descriptor(&format!("O:{}D:P(A;;GA;;;WD)", user.sid_text));
    let _handle =
        Handle::new(unsafe { CreateMutexW(&attributes(&sd), 0, namespace.mutex("live").as_ptr()) })
            .unwrap();
    assert_eq!(
        instance::gui_is_alive(&namespace).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn client_refuses_untrusted_pipe_acl_before_sending_and_real_deny_acl_blocks_open() {
    let user = UserSecurity::current().unwrap();
    for denied in [false, true] {
        let namespace = Namespace::isolated();
        let acl = if denied {
            format!("(D;;GA;;;{})(A;;GA;;;WD)", user.sid_text)
        } else {
            "(A;;GA;;;WD)".into()
        };
        let sd = descriptor(&format!("O:{}D:P{acl}", user.sid_text));
        let pipe = Handle::new(unsafe {
            CreateNamedPipeW(
                namespace.pipe().as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                &attributes(&sd),
            )
        })
        .unwrap();
        let error = PipeClient::try_connect(&namespace).err().unwrap();
        if denied {
            assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
        } else {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            let mut available = 0;
            let peeked = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    pipe.0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut available,
                    ptr::null_mut(),
                )
            };
            if peeked == 0 {
                assert_eq!(
                    io::Error::last_os_error().raw_os_error(),
                    Some(windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE as i32)
                );
            }
            assert_eq!(available, 0);
        }
    }
}
