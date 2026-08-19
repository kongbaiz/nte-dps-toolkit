use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::{
    ffi::{OsStr, OsString, c_void},
    ptr::{self, NonNull},
};

use windows_sys::Win32::Foundation::{E_INVALIDARG, E_POINTER};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows_sys::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT,
    FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, SIGDN_FILESYSPATH,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

const IID_IFILE_OPEN_DIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
const IID_IFILE_SAVE_DIALOG: GUID = GUID::from_u128(0x84bccd23_5fde_4cdb_aea4_af64b83d78ab);
const DIALOG_CANCELLED_HRESULT: HRESULT = 0x8007_04c7_u32 as HRESULT;
const MAX_WINDOWS_PATH_UTF16_UNITS: usize = 32_768;
const OPEN_FILE_OPTIONS: u32 =
    FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;
const SAVE_FILE_OPTIONS: u32 =
    FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT;
const FOLDER_OPTIONS: u32 =
    FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR;

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

/// User-visible filter labels are supplied by the desktop adapter so this
/// platform boundary remains independent of localization storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogFilterLabels {
    pub all_files: String,
    pub json_files: String,
    pub ini_files: String,
    pub pcapng_files: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogError {
    NativeHresult(HRESULT),
}

impl FileDialogError {
    pub const fn hresult(self) -> HRESULT {
        match self {
            Self::NativeHresult(hresult) => hresult,
        }
    }
}

pub type FolderDialogError = FileDialogError;

pub fn is_current_process_window(owner: isize) -> bool {
    if owner == 0 {
        return false;
    }
    let window = owner as *mut c_void;
    // SAFETY: IsWindow accepts opaque/stale HWND values and reports invalid
    // handles without dereferencing application memory.
    if unsafe { IsWindow(window) } == 0 {
        return false;
    }
    let mut process_id = 0;
    // SAFETY: `process_id` is writable and the HWND passed the immediately
    // preceding IsWindow check. Zero is handled as failure.
    let thread_id = unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    thread_id != 0 && process_id == std::process::id()
}

pub fn choose_folder(owner: isize, title: &str) -> Result<FolderDialogOutcome, FolderDialogError> {
    let _apartment = ComApartment::initialize()?;
    let dialog = create_file_open_dialog()?;
    let dialog_vtable = dialog.vtable::<FileOpenDialogVtable>()?;
    configure_dialog(&dialog, &dialog_vtable.base, title, FOLDER_OPTIONS, None)?;

    match show_and_resolve_path(&dialog, &dialog_vtable.base, owner)? {
        Some(path) => Ok(FolderDialogOutcome::Selected(path)),
        None => Ok(FolderDialogOutcome::Cancelled),
    }
}

fn dialog_was_selected(hresult: HRESULT) -> Result<bool, FileDialogError> {
    if hresult >= 0 {
        Ok(true)
    } else if hresult == DIALOG_CANCELLED_HRESULT {
        Ok(false)
    } else {
        Err(FileDialogError::NativeHresult(hresult))
    }
}

fn check_hresult(hresult: HRESULT) -> Result<(), FileDialogError> {
    if hresult >= 0 {
        Ok(())
    } else {
        Err(FileDialogError::NativeHresult(hresult))
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, FileDialogError> {
        let flags = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: the reserved pointer is null as required. A successful call is
        // paired with exactly one CoUninitialize by ComApartment::drop.
        check_hresult(unsafe { CoInitializeEx(ptr::null(), flags) })?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: construction is possible only after a successful CoInitializeEx
        // on this same synchronous worker thread.
        unsafe { CoUninitialize() };
    }
}

struct ComObject {
    pointer: NonNull<c_void>,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

impl ComObject {
    /// Takes ownership of one COM reference returned through an interface out pointer.
    unsafe fn from_raw(pointer: *mut c_void) -> Result<Self, FileDialogError> {
        let pointer = NonNull::new(pointer).ok_or(FileDialogError::NativeHresult(E_POINTER))?;
        // SAFETY: every COM interface begins with a non-null vtable pointer whose
        // first three entries use the IUnknown ABI.
        let vtable = unsafe { *pointer.as_ptr().cast::<*const UnknownVtable>() };
        let Some(vtable) = (unsafe { vtable.as_ref() }) else {
            return Err(FileDialogError::NativeHresult(E_POINTER));
        };
        Ok(Self {
            pointer,
            release: vtable.release,
        })
    }

    fn as_raw(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }

    fn vtable<T>(&self) -> Result<&T, FileDialogError> {
        // SAFETY: `self.pointer` is a live COM interface pointer. The requested
        // vtable type is selected only for the interface IID used to obtain it.
        let pointer = unsafe { *self.pointer.as_ptr().cast::<*const T>() };
        unsafe { pointer.as_ref() }.ok_or(FileDialogError::NativeHresult(E_POINTER))
    }
}

impl Drop for ComObject {
    fn drop(&mut self) {
        // SAFETY: ComObject owns exactly one live COM reference and stores the
        // Release entry read from that interface's IUnknown prefix.
        unsafe { (self.release)(self.pointer.as_ptr()) };
    }
}

fn create_file_open_dialog() -> Result<ComObject, FileDialogError> {
    create_file_dialog(&FileOpenDialog, &IID_IFILE_OPEN_DIALOG)
}

fn create_file_save_dialog() -> Result<ComObject, FileDialogError> {
    create_file_dialog(&FileSaveDialog, &IID_IFILE_SAVE_DIALOG)
}

fn create_file_dialog(class_id: &GUID, interface_id: &GUID) -> Result<ComObject, FileDialogError> {
    let mut raw_dialog = ptr::null_mut();
    // SAFETY: CLSID/IID point to static GUIDs, aggregation is not requested, and
    // `raw_dialog` is a writable interface out pointer.
    check_hresult(unsafe {
        CoCreateInstance(
            class_id,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            interface_id,
            &mut raw_dialog,
        )
    })?;
    // SAFETY: a successful CoCreateInstance returns one owned interface reference.
    unsafe { ComObject::from_raw(raw_dialog) }
}

struct CoTaskMemWide(PWSTR);

impl CoTaskMemWide {
    fn new(pointer: PWSTR) -> Self {
        Self(pointer)
    }

    fn to_path_buf(&self) -> Result<PathBuf, FileDialogError> {
        // SAFETY: this wrapper owns a live NUL-terminated string returned by
        // IShellItem::GetDisplayName for the duration of the conversion.
        unsafe { path_buf_from_wide_pointer(self.0) }
    }
}

unsafe fn path_buf_from_wide_pointer(pointer: *const u16) -> Result<PathBuf, FileDialogError> {
    if pointer.is_null() {
        return Err(FileDialogError::NativeHresult(E_POINTER));
    }
    let mut length = 0;
    // SAFETY: IFileDialog returns a CoTaskMem-allocated NUL-terminated PWSTR.
    // The explicit NT path bound prevents an unbounded scan if that contract
    // is violated by the external shell boundary.
    while length < MAX_WINDOWS_PATH_UTF16_UNITS && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    if length == MAX_WINDOWS_PATH_UTF16_UNITS {
        return Err(FileDialogError::NativeHresult(E_INVALIDARG));
    }
    // SAFETY: the scan above established `length` initialized UTF-16 units
    // before the terminating NUL within the documented Windows path bound.
    let units = unsafe { std::slice::from_raw_parts(pointer, length) };
    Ok(PathBuf::from(OsString::from_wide(units)))
}

impl Drop for CoTaskMemWide {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: GetDisplayName allocates this pointer with the COM task
            // allocator and ownership has not been transferred elsewhere.
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }
}

#[repr(C)]
#[allow(dead_code)]
struct UnknownVtable {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
#[allow(dead_code)]
struct ModalWindowVtable {
    base: UnknownVtable,
    show: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
#[allow(dead_code)]
struct FileDialogVtable {
    base: ModalWindowVtable,
    set_file_types:
        unsafe extern "system" fn(*mut c_void, u32, *const COMDLG_FILTERSPEC) -> HRESULT,
    set_file_type_index: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_file_type_index: usize,
    advise: usize,
    unadvise: usize,
    set_options: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    set_default_folder: usize,
    set_folder: usize,
    get_folder: usize,
    get_current_selection: usize,
    set_file_name: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_file_name: usize,
    set_title: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    set_ok_button_label: usize,
    set_file_name_label: usize,
    get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    add_place: usize,
    set_default_extension: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    close: usize,
    set_client_guid: usize,
    clear_client_data: usize,
    set_filter: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct FileOpenDialogVtable {
    base: FileDialogVtable,
    get_results: usize,
    get_selected_items: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct FileSaveDialogVtable {
    base: FileDialogVtable,
    set_save_as_item: usize,
    set_properties: usize,
    set_collected_properties: usize,
    get_properties: usize,
    apply_properties: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct ShellItemVtable {
    base: UnknownVtable,
    bind_to_handler: usize,
    get_parent: usize,
    get_display_name: unsafe extern "system" fn(*mut c_void, i32, *mut PWSTR) -> HRESULT,
    get_attributes: usize,
    compare: usize,
}

pub fn open_directory(path: &std::path::Path) -> Result<(), std::io::Error> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
}

pub fn choose_json_open_path(
    owner: isize,
    title: &str,
    labels: &FileDialogFilterLabels,
) -> Result<OpenFileDialogOutcome, FileDialogError> {
    choose_open_path(owner, title, &json_filters(labels))
}

pub fn choose_pcapng_open_path(
    owner: isize,
    title: &str,
    labels: &FileDialogFilterLabels,
) -> Result<OpenFileDialogOutcome, FileDialogError> {
    choose_open_path(owner, title, &pcapng_filters(labels))
}

pub fn choose_ini_open_path(
    owner: isize,
    title: &str,
    labels: &FileDialogFilterLabels,
) -> Result<OpenFileDialogOutcome, FileDialogError> {
    choose_open_path(owner, title, &ini_filters(labels))
}

fn choose_open_path(
    owner: isize,
    title: &str,
    filters: &[DialogFilter<'_>],
) -> Result<OpenFileDialogOutcome, FileDialogError> {
    let _apartment = ComApartment::initialize()?;
    let dialog = create_file_open_dialog()?;
    let dialog_vtable = dialog.vtable::<FileOpenDialogVtable>()?;
    let native_filters = NativeFilterSet::new(filters)?;
    configure_dialog(
        &dialog,
        &dialog_vtable.base,
        title,
        OPEN_FILE_OPTIONS,
        Some(&native_filters),
    )?;

    match show_and_resolve_path(&dialog, &dialog_vtable.base, owner)? {
        Some(path) => Ok(OpenFileDialogOutcome::Selected(path)),
        None => Ok(OpenFileDialogOutcome::Cancelled),
    }
}

pub fn choose_json_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
    labels: &FileDialogFilterLabels,
) -> Result<SaveFileDialogOutcome, FileDialogError> {
    choose_save_path(
        owner,
        title,
        default_file_name,
        &json_filters(labels),
        "json",
    )
}

pub fn choose_pcapng_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
    labels: &FileDialogFilterLabels,
) -> Result<SaveFileDialogOutcome, FileDialogError> {
    choose_save_path(
        owner,
        title,
        default_file_name,
        &pcapng_filters(labels),
        "pcapng",
    )
}

fn choose_save_path(
    owner: isize,
    title: &str,
    default_file_name: &str,
    filters: &[DialogFilter<'_>],
    default_extension: &str,
) -> Result<SaveFileDialogOutcome, FileDialogError> {
    let _apartment = ComApartment::initialize()?;
    let dialog = create_file_save_dialog()?;
    let dialog_vtable = dialog.vtable::<FileSaveDialogVtable>()?;
    let native_filters = NativeFilterSet::new(filters)?;
    configure_dialog(
        &dialog,
        &dialog_vtable.base,
        title,
        SAVE_FILE_OPTIONS,
        Some(&native_filters),
    )?;

    configure_save_defaults(
        &dialog,
        &dialog_vtable.base,
        default_file_name,
        default_extension,
    )?;

    match show_and_resolve_path(&dialog, &dialog_vtable.base, owner)? {
        Some(path) => Ok(SaveFileDialogOutcome::Selected(path)),
        None => Ok(SaveFileDialogOutcome::Cancelled),
    }
}

fn configure_dialog(
    dialog: &ComObject,
    vtable: &FileDialogVtable,
    title: &str,
    required_options: u32,
    filters: Option<&NativeFilterSet>,
) -> Result<(), FileDialogError> {
    let mut options = 0;
    // SAFETY: the dialog is a live IFileDialog and `options` is writable.
    check_hresult(unsafe { (vtable.get_options)(dialog.as_raw(), &mut options) })?;
    // SAFETY: the dialog remains live and the option mask uses documented FOS bits.
    check_hresult(unsafe { (vtable.set_options)(dialog.as_raw(), options | required_options) })?;

    let title = wide_string(title)?;
    // SAFETY: `title` is NUL-terminated and remains alive through the call.
    check_hresult(unsafe { (vtable.set_title)(dialog.as_raw(), title.as_ptr()) })?;

    if let Some(filters) = filters {
        // SAFETY: every COMDLG_FILTERSPEC points into a NUL-terminated allocation
        // owned by `filters`, which outlives this call and the dialog Show call.
        check_hresult(unsafe {
            (vtable.set_file_types)(dialog.as_raw(), filters.count, filters.native.as_ptr())
        })?;
        // COM file-type indexes are one-based; select the first declared filter.
        // SAFETY: configure_dialog rejects an empty filter set.
        check_hresult(unsafe { (vtable.set_file_type_index)(dialog.as_raw(), 1) })?;
    }
    Ok(())
}

fn show_and_resolve_path(
    dialog: &ComObject,
    vtable: &FileDialogVtable,
    owner: isize,
) -> Result<Option<PathBuf>, FileDialogError> {
    // SAFETY: the owner was re-resolved and validated by the Tauri blocking
    // adapter immediately before this synchronous modal COM call.
    let show_result = unsafe { (vtable.base.show)(dialog.as_raw(), owner as *mut c_void) };
    if !dialog_was_selected(show_result)? {
        return Ok(None);
    }

    let mut raw_item = ptr::null_mut();
    // SAFETY: `raw_item` is a writable out pointer and the dialog remains live.
    check_hresult(unsafe { (vtable.get_result)(dialog.as_raw(), &mut raw_item) })?;
    // SAFETY: successful GetResult returns one owned IShellItem reference.
    let item = unsafe { ComObject::from_raw(raw_item) }?;
    let item_vtable = item.vtable::<ShellItemVtable>()?;
    let mut raw_path: PWSTR = ptr::null_mut();
    // SAFETY: SIGDN_FILESYSPATH returns a shell-allocated, NUL-terminated path.
    let path_result =
        unsafe { (item_vtable.get_display_name)(item.as_raw(), SIGDN_FILESYSPATH, &mut raw_path) };
    check_hresult(path_result)?;
    let path = CoTaskMemWide::new(raw_path);
    Ok(Some(path.to_path_buf()?))
}

fn configure_save_defaults(
    dialog: &ComObject,
    vtable: &FileDialogVtable,
    default_file_name: &str,
    default_extension: &str,
) -> Result<(), FileDialogError> {
    let default_file_name = wide_string(default_file_name)?;
    // SAFETY: the dialog is live, and the NUL-terminated filename allocation
    // remains alive through this synchronous COM call.
    check_hresult(unsafe { (vtable.set_file_name)(dialog.as_raw(), default_file_name.as_ptr()) })?;
    let default_extension = wide_string(default_extension)?;
    // SAFETY: the dialog is live, and the NUL-terminated extension allocation
    // remains alive through this synchronous COM call.
    check_hresult(unsafe {
        (vtable.set_default_extension)(dialog.as_raw(), default_extension.as_ptr())
    })
}

fn wide_string(value: &str) -> Result<Vec<u16>, FileDialogError> {
    let mut units = OsStr::new(value).encode_wide().collect::<Vec<_>>();
    if units.len() >= MAX_WINDOWS_PATH_UTF16_UNITS || units.contains(&0) {
        return Err(FileDialogError::NativeHresult(E_INVALIDARG));
    }
    units.push(0);
    Ok(units)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DialogFilter<'a> {
    name: &'a str,
    pattern: &'static str,
}

fn json_filters(labels: &FileDialogFilterLabels) -> [DialogFilter<'_>; 2] {
    [
        DialogFilter {
            name: &labels.json_files,
            pattern: "*.json",
        },
        all_files_filter(labels),
    ]
}

fn ini_filters(labels: &FileDialogFilterLabels) -> [DialogFilter<'_>; 2] {
    [
        DialogFilter {
            name: &labels.ini_files,
            pattern: "*.ini",
        },
        all_files_filter(labels),
    ]
}

fn pcapng_filters(labels: &FileDialogFilterLabels) -> [DialogFilter<'_>; 2] {
    [
        DialogFilter {
            name: &labels.pcapng_files,
            pattern: "*.pcapng",
        },
        all_files_filter(labels),
    ]
}

fn all_files_filter(labels: &FileDialogFilterLabels) -> DialogFilter<'_> {
    DialogFilter {
        name: &labels.all_files,
        pattern: "*.*",
    }
}

struct NativeFilterSet {
    _names: Vec<Vec<u16>>,
    _patterns: Vec<Vec<u16>>,
    native: Vec<COMDLG_FILTERSPEC>,
    count: u32,
}

impl NativeFilterSet {
    fn new(filters: &[DialogFilter<'_>]) -> Result<Self, FileDialogError> {
        if filters.is_empty() {
            return Err(FileDialogError::NativeHresult(E_INVALIDARG));
        }
        let count = u32::try_from(filters.len())
            .map_err(|_| FileDialogError::NativeHresult(E_INVALIDARG))?;
        let names = filters
            .iter()
            .map(|filter| wide_string(filter.name))
            .collect::<Result<Vec<_>, _>>()?;
        let patterns = filters
            .iter()
            .map(|filter| wide_string(filter.pattern))
            .collect::<Result<Vec<_>, _>>()?;
        let native = names
            .iter()
            .zip(&patterns)
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: name.as_ptr(),
                pszSpec: pattern.as_ptr(),
            })
            .collect();
        Ok(Self {
            _names: names,
            _patterns: patterns,
            native,
            count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter_labels() -> FileDialogFilterLabels {
        FileDialogFilterLabels {
            all_files: "localized all files".to_owned(),
            json_files: "localized JSON files".to_owned(),
            ini_files: "localized INI files".to_owned(),
            pcapng_files: "localized PCAPNG files".to_owned(),
        }
    }

    #[test]
    fn native_dialog_strings_are_terminated() {
        assert_eq!(
            wide_string("nte-team-dps.json")
                .expect("valid filename")
                .last(),
            Some(&0)
        );
        assert!(matches!(
            wide_string("bad\0name.json"),
            Err(FileDialogError::NativeHresult(E_INVALIDARG))
        ));
        assert!(matches!(
            wide_string(&"a".repeat(MAX_WINDOWS_PATH_UTF16_UNITS)),
            Err(FileDialogError::NativeHresult(E_INVALIDARG))
        ));
        let labels = filter_labels();
        for filters in [
            json_filters(&labels),
            ini_filters(&labels),
            pcapng_filters(&labels),
        ] {
            let native = NativeFilterSet::new(&filters).expect("valid native filters");
            assert_eq!(native.count as usize, filters.len());
            assert!(native._names.iter().all(|value| value.last() == Some(&0)));
            assert!(
                native
                    ._patterns
                    .iter()
                    .all(|value| value.last() == Some(&0))
            );
            assert!(
                native
                    .native
                    .iter()
                    .zip(&native._names)
                    .zip(&native._patterns)
                    .all(|((spec, name), pattern)| spec.pszName == name.as_ptr()
                        && spec.pszSpec == pattern.as_ptr())
            );
        }
        assert!(matches!(
            NativeFilterSet::new(&[]),
            Err(FileDialogError::NativeHresult(E_INVALIDARG))
        ));
    }

    #[test]
    fn folder_picker_uses_dynamic_com_path_resolution() {
        let source = include_str!("file_dialog.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        let legacy_browse = ["SHBrowse", "ForFolderW"].concat();
        let legacy_resolve = ["SHGetPath", "FromIDListW"].concat();
        let max_path_buffer = ["[0_u16; ", "260]"].concat();

        assert!(production.contains("FOS_PICKFOLDERS"));
        assert!(production.contains("SIGDN_FILESYSPATH"));
        assert!(production.contains("CoTaskMemFree"));
        assert!(production.contains("IsWindow"));
        assert!(production.contains("GetWindowThreadProcessId"));
        assert!(!production.contains(&legacy_browse));
        assert!(!production.contains(&legacy_resolve));
        assert!(!production.contains(&max_path_buffer));
    }

    #[test]
    fn every_file_picker_uses_the_com_dialog_boundary() {
        let source = include_str!("file_dialog.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        let legacy_open = ["GetOpen", "FileNameW"].concat();
        let legacy_save = ["GetSave", "FileNameW"].concat();
        let legacy_error = ["CommDlg", "ExtendedError"].concat();
        let legacy_structure = ["OPENFILE", "NAMEW"].concat();

        assert!(production.contains("FileOpenDialog"));
        assert!(production.contains("FileSaveDialog"));
        assert!(production.contains("COMDLG_FILTERSPEC"));
        assert!(production.contains("set_file_types"));
        assert!(production.contains("set_file_name"));
        assert!(production.contains("set_default_extension"));
        assert!(!production.contains(&legacy_open));
        assert!(!production.contains(&legacy_save));
        assert!(!production.contains(&legacy_error));
        assert!(!production.contains(&legacy_structure));
    }

    #[test]
    fn native_filter_labels_are_supplied_by_the_localized_adapter() {
        let source = include_str!("file_dialog.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");

        for hard_coded_label in [
            "name: \"All files (*.*)\"",
            "name: \"JSON files (*.json)\"",
            "name: \"INI files (*.ini)\"",
            "name: \"PCAPNG files (*.pcapng)\"",
        ] {
            assert!(
                !production.contains(hard_coded_label),
                "platform dialog code must not own user-visible label {hard_coded_label}"
            );
        }
        assert!(production.contains("FileDialogFilterLabels"));
    }

    #[test]
    fn dialog_options_preserve_open_save_and_folder_semantics() {
        assert_eq!(OPEN_FILE_OPTIONS & FOS_FILEMUSTEXIST, FOS_FILEMUSTEXIST);
        assert_eq!(OPEN_FILE_OPTIONS & FOS_PATHMUSTEXIST, FOS_PATHMUSTEXIST);
        assert_eq!(OPEN_FILE_OPTIONS & FOS_PICKFOLDERS, 0);
        assert_eq!(SAVE_FILE_OPTIONS & FOS_OVERWRITEPROMPT, FOS_OVERWRITEPROMPT);
        assert_eq!(SAVE_FILE_OPTIONS & FOS_PATHMUSTEXIST, FOS_PATHMUSTEXIST);
        assert_eq!(SAVE_FILE_OPTIONS & FOS_FILEMUSTEXIST, 0);
        assert_eq!(FOLDER_OPTIONS & FOS_PICKFOLDERS, FOS_PICKFOLDERS);
        assert_eq!(FOLDER_OPTIONS & FOS_FORCEFILESYSTEM, FOS_FORCEFILESYSTEM);
        assert_eq!(OPEN_FILE_OPTIONS & FOS_NOCHANGEDIR, FOS_NOCHANGEDIR);
        assert_eq!(SAVE_FILE_OPTIONS & FOS_NOCHANGEDIR, FOS_NOCHANGEDIR);
    }

    #[test]
    fn dialog_show_distinguishes_selection_cancellation_and_hresult() {
        assert_eq!(dialog_was_selected(0), Ok(true));
        assert_eq!(dialog_was_selected(DIALOG_CANCELLED_HRESULT), Ok(false));
        assert_eq!(
            dialog_was_selected(E_INVALIDARG),
            Err(FileDialogError::NativeHresult(E_INVALIDARG))
        );
    }

    #[test]
    fn com_dialog_objects_accept_open_and_save_configuration_without_showing() {
        let _apartment = ComApartment::initialize().expect("STA COM apartment");
        let labels = filter_labels();
        let native_filters = NativeFilterSet::new(&json_filters(&labels)).expect("native filters");

        let open_dialog = create_file_open_dialog().expect("IFileOpenDialog");
        let open_vtable = open_dialog
            .vtable::<FileOpenDialogVtable>()
            .expect("IFileOpenDialog vtable");
        configure_dialog(
            &open_dialog,
            &open_vtable.base,
            "Open capture",
            OPEN_FILE_OPTIONS,
            Some(&native_filters),
        )
        .expect("configure open dialog");

        let save_dialog = create_file_save_dialog().expect("IFileSaveDialog");
        let save_vtable = save_dialog
            .vtable::<FileSaveDialogVtable>()
            .expect("IFileSaveDialog vtable");
        configure_dialog(
            &save_dialog,
            &save_vtable.base,
            "Save capture",
            SAVE_FILE_OPTIONS,
            Some(&native_filters),
        )
        .expect("configure save dialog");
        configure_save_defaults(&save_dialog, &save_vtable.base, "nte-capture.json", "json")
            .expect("configure save defaults");
    }

    #[test]
    fn dynamic_folder_path_conversion_preserves_more_than_max_path() {
        let path = format!(r"C:\\{}\\game", "a".repeat(300));
        let mut wide = OsStr::new(&path).encode_wide().collect::<Vec<_>>();
        wide.push(0);

        // SAFETY: `wide` is initialized and explicitly NUL-terminated.
        let converted = unsafe { path_buf_from_wide_pointer(wide.as_ptr()) }
            .expect("long folder path conversion");
        assert_eq!(converted, PathBuf::from(path));
    }

    #[test]
    fn dynamic_folder_path_conversion_rejects_invalid_external_buffers() {
        // SAFETY: null is passed specifically to exercise the checked boundary.
        assert_eq!(
            unsafe { path_buf_from_wide_pointer(ptr::null()) },
            Err(FileDialogError::NativeHresult(E_POINTER))
        );

        let unterminated = vec![u16::from(b'a'); MAX_WINDOWS_PATH_UTF16_UNITS];
        // SAFETY: the vector contains exactly the maximum number of initialized
        // units, so the bounded scan never reads outside the allocation.
        assert_eq!(
            unsafe { path_buf_from_wide_pointer(unterminated.as_ptr()) },
            Err(FileDialogError::NativeHresult(E_INVALIDARG))
        );
    }
}
