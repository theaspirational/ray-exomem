use crate::{ffi, storage::RayObj};
use anyhow::{anyhow, Context, Result};
use libc::c_char;
use std::{
    ffi::{CStr, CString},
    fs,
    path::Path,
};

pub struct RayforceEngine {
    runtime: *mut ffi::ray_runtime_t,
}

impl RayforceEngine {
    pub fn new() -> Result<Self> {
        let runtime = unsafe { ffi::ray_runtime_create(0, std::ptr::null_mut()) };
        if runtime.is_null() {
            return Err(anyhow!("rayforce2 runtime initialization failed"));
        }

        unsafe {
            ffi::ray_fmt_set_precision(6);
            ffi::ray_fmt_set_width(120);
        }

        Ok(Self { runtime })
    }

    /// Create a runtime that loads the symbol table before registering
    /// builtins, so persisted symbol IDs keep their slots across restarts.
    pub fn new_with_sym(sym_path: &Path) -> Result<Self> {
        let c_sym = CString::new(sym_path.to_str().unwrap_or(""))
            .context("sym_path contains interior NUL")?;
        let runtime = unsafe { ffi::ray_runtime_create_with_sym(c_sym.as_ptr()) };
        if runtime.is_null() {
            return Err(anyhow!("rayforce2 runtime initialization failed"));
        }

        unsafe {
            ffi::ray_fmt_set_precision(6);
            ffi::ray_fmt_set_width(120);
        }

        Ok(Self { runtime })
    }

    pub fn version(&self) -> String {
        unsafe {
            let ptr = ffi::ray_version_string();
            if ptr.is_null() {
                return "unknown".into();
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }

    pub fn eval_raw(&self, source: &str) -> Result<RayObj> {
        let c_source = CString::new(source).context("source contains interior NUL byte")?;

        unsafe {
            ffi::ray_error_clear();
            let raw = ffi::ray_eval_str(c_source.as_ptr());

            let err_msg = ffi::ray_error_msg();
            if !err_msg.is_null() {
                let message = CStr::from_ptr(err_msg).to_string_lossy().into_owned();
                if !raw.is_null() {
                    ffi::ray_release(raw);
                }
                return Err(anyhow!(message));
            }

            if raw.is_null() {
                return Err(anyhow!("ray_eval_str returned a null result"));
            }

            RayObj::from_raw(raw)
        }
    }

    pub fn format_obj(&self, obj: &RayObj) -> Result<String> {
        unsafe {
            let formatted = ffi::ray_fmt(obj.as_ptr(), 2);
            if formatted.is_null() {
                return Err(anyhow!("ray_fmt returned a null result"));
            }

            let ptr = ffi::ray_str_ptr(formatted);
            let len = ffi::ray_str_len(formatted);
            let output = if ptr.is_null() {
                String::new()
            } else {
                let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
                String::from_utf8_lossy(bytes).into_owned()
            };

            ffi::ray_release(formatted);
            let trimmed = output.trim_start();
            if trimmed
                .get(..6)
                .map(|prefix| prefix.eq_ignore_ascii_case("error:"))
                .unwrap_or(false)
            {
                return Err(anyhow!(trimmed[6..].trim_start().to_owned()));
            }

            Ok(output)
        }
    }

    pub fn eval(&self, source: &str) -> Result<String> {
        let raw = self.eval_raw(source)?;
        self.format_obj(&raw)
    }

    pub fn bind_named_db(&self, sym_id: i64, table: &RayObj) -> Result<()> {
        let err = unsafe { ffi::ray_env_set(sym_id, table.as_ptr()) };
        if err != ffi::RAY_OK {
            return Err(anyhow!("ray_env_set failed with error code {}", err));
        }
        Ok(())
    }

    pub fn get_named_db(&self, sym_id: i64) -> Result<Option<RayObj>> {
        unsafe {
            let ptr = ffi::ray_env_get(sym_id);
            if ptr.is_null() {
                return Ok(None);
            }
            ffi::ray_retain(ptr);
            Ok(Some(RayObj::from_raw(ptr)?))
        }
    }

    /// After `ray_env_set` retains bound tables, the C env holds extra refcounts. If
    /// [`Self::bind_named_db`] / `eval` fail mid-restore, unwinding Rust `RayObj` drops can fault in
    /// `ray_release`. Call this before dropping `RayObj` values that were bound, then run
    /// `restore_runtime` again to re-bind from current `DaemonState`.
    pub fn reconcile_lang_env(&self) -> Result<()> {
        unsafe {
            ffi::ray_env_destroy();
        }
        let err = unsafe { ffi::ray_lang_init() };
        if err != ffi::RAY_OK {
            return Err(anyhow!(
                "ray_lang_init failed after ray_env_destroy (code {})",
                err
            ));
        }
        Ok(())
    }

    pub fn eval_file(&self, path: &Path) -> Result<String> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read source file: {}", path.display()))?;
        self.eval(&source)
    }
}

impl Drop for RayforceEngine {
    fn drop(&mut self) {
        unsafe {
            if !self.runtime.is_null() {
                ffi::ray_runtime_destroy(self.runtime);
            }
        }
    }
}

unsafe impl Send for RayforceEngine {}
unsafe impl Sync for RayforceEngine {}

pub fn rayforce_version() -> String {
    unsafe {
        let ptr = ffi::ray_version_string();
        if ptr.is_null() {
            "unknown".into()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

#[allow(dead_code)]
fn _assert_c_char(_: c_char) {}
