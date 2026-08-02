use std::ffi::{OsStr, OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    BIF_RETURNONLYFSDIRS, BROWSEINFOW, ILFree, SHBrowseForFolderW, SHGetPathFromIDListW,
};

pub enum SaveFileDialogOutcome {
    Selected(PathBuf),
    Cancelled,
}

pub enum OpenFileDialogOutcome {
    Selected(PathBuf),
    Cancelled,
}

pub enum FolderDialogOutcome {
    Selected(PathBuf),
    Cancelled,
}

pub fn choose_folder(owner: isize, title: &str) -> Result<FolderDialogOutcome, u32> {
    let title = wide_string(title);
    let mut display_name = [0_u16; 260];
    let dialog = BROWSEINFOW {
        hwndOwner: owner as *mut c_void,
        pszDisplayName: display_name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS,
        ..BROWSEINFOW::default()
    };
    // SAFETY: BROWSEINFOW contains valid buffers and pointers that remain
    // alive for the duration of the synchronous shell dialog call.
    let item = unsafe { SHBrowseForFolderW(&dialog) };
    if item.is_null() {
        return Ok(FolderDialogOutcome::Cancelled);
    }
    let mut path = [0_u16; 260];
    // SAFETY: `item` was allocated by the shell and remains valid until ILFree;
    // `path` is writable and sized to the API's documented MAX_PATH buffer.
    let resolved = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) } != 0;
    // SAFETY: `item` was returned by SHBrowseForFolderW and is released once.
    unsafe { ILFree(item) };
    if !resolved {
        return Err(1);
    }
    let path_len = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    Ok(FolderDialogOutcome::Selected(PathBuf::from(
        OsString::from_wide(&path[..path_len]),
    )))
}

pub fn open_directory(path: &std::path::Path) -> Result<(), std::io::Error> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
}

pub fn choose_json_open_path(owner: isize, title: &str) -> Result<OpenFileDialogOutcome, u32> {
    choose_open_path(owner, title, json_filter())
}

pub fn choose_pcapng_open_path(owner: isize, title: &str) -> Result<OpenFileDialogOutcome, u32> {
    choose_open_path(owner, title, pcapng_filter())
}

pub fn choose_ini_open_path(owner: isize, title: &str) -> Result<OpenFileDialogOutcome, u32> {
    choose_open_path(owner, title, ini_filter())
}

fn choose_open_path(
    owner: isize,
    title: &str,
    filter: Vec<u16>,
) -> Result<OpenFileDialogOutcome, u32> {
    let mut file_buffer = vec![0_u16; 32_768];
    let title = wide_string(title);
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as *mut c_void,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file_buffer.as_mut_ptr(),
        nMaxFile: file_buffer.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST,
        ..OPENFILENAMEW::default()
    };

    // SAFETY: OPENFILENAMEW points to writable, NUL-terminated buffers that
    // remain alive for the duration of the synchronous native dialog call.
    if unsafe { GetOpenFileNameW(&mut dialog) } != 0 {
        let path_len = file_buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file_buffer.len());
        return Ok(OpenFileDialogOutcome::Selected(PathBuf::from(
            OsString::from_wide(&file_buffer[..path_len]),
        )));
    }

    // SAFETY: CommDlgExtendedError reports the last common-dialog failure.
    let error = unsafe { CommDlgExtendedError() };
    if error == 0 {
        Ok(OpenFileDialogOutcome::Cancelled)
    } else {
        Err(error)
    }
}

pub fn choose_json_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
) -> Result<SaveFileDialogOutcome, u32> {
    choose_save_path(owner, title, default_file_name, json_filter(), "json")
}

pub fn choose_pcapng_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
) -> Result<SaveFileDialogOutcome, u32> {
    choose_save_path(owner, title, default_file_name, pcapng_filter(), "pcapng")
}

fn choose_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
    filter: Vec<u16>,
    default_extension: &str,
) -> Result<SaveFileDialogOutcome, u32> {
    let mut file_buffer = vec![0_u16; 32_768];
    let default_file_name = wide_string(default_file_name);
    let copy_len = default_file_name
        .len()
        .saturating_sub(1)
        .min(file_buffer.len() - 1);
    file_buffer[..copy_len].copy_from_slice(&default_file_name[..copy_len]);

    let title = wide_string(title);
    let default_extension = wide_string(default_extension);
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as *mut c_void,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file_buffer.as_mut_ptr(),
        nMaxFile: file_buffer.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        lpstrDefExt: default_extension.as_ptr(),
        ..OPENFILENAMEW::default()
    };

    // SAFETY: OPENFILENAMEW points to writable, NUL-terminated buffers that
    // remain alive for the duration of the synchronous native dialog call.
    if unsafe { GetSaveFileNameW(&mut dialog) } != 0 {
        let path_len = file_buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file_buffer.len());
        return Ok(SaveFileDialogOutcome::Selected(PathBuf::from(
            OsString::from_wide(&file_buffer[..path_len]),
        )));
    }

    // SAFETY: CommDlgExtendedError takes no arguments and reports the last
    // common-dialog failure for this thread. Zero means the user cancelled.
    let error = unsafe { CommDlgExtendedError() };
    if error == 0 {
        Ok(SaveFileDialogOutcome::Cancelled)
    } else {
        Err(error)
    }
}

fn wide_string(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn json_filter() -> Vec<u16> {
    OsStr::new("JSON files (*.json)\0*.json\0All files (*.*)\0*.*\0")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn ini_filter() -> Vec<u16> {
    OsStr::new("INI files (*.ini)\0*.ini\0All files (*.*)\0*.*\0")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn pcapng_filter() -> Vec<u16> {
    OsStr::new("PCAPNG files (*.pcapng)\0*.pcapng\0All files (*.*)\0*.*\0")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_dialog_strings_are_terminated() {
        assert_eq!(wide_string("nte-team-dps.json").last(), Some(&0));
        let filter = json_filter();
        assert_eq!(&filter[filter.len() - 2..], &[0, 0]);
        let filter = ini_filter();
        assert_eq!(&filter[filter.len() - 2..], &[0, 0]);
        let filter = pcapng_filter();
        assert_eq!(&filter[filter.len() - 2..], &[0, 0]);
    }
}
