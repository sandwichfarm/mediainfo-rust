use libloading::{Library, Symbol};
use std::ffi::c_void;
type NewFn = unsafe extern "C" fn() -> *mut c_void;
type DeleteFn = unsafe extern "C" fn(*mut c_void);
type OpenFn = unsafe extern "C" fn(*mut c_void, *const u32) -> usize;
type CloseFn = unsafe extern "C" fn(*mut c_void);
type InformFn = unsafe extern "C" fn(*mut c_void, usize) -> *const u32;
type GetIFn = unsafe extern "C" fn(*mut c_void, usize, usize, usize, usize) -> *const u32;
type OptionFn = unsafe extern "C" fn(*mut c_void, *const u32, *const u32) -> *const u32;
type CountGetFn = unsafe extern "C" fn(*mut c_void, usize, usize) -> usize;
fn w(s: &str) -> Vec<u32> { s.chars().map(|c| c as u32).chain(std::iter::once(0)).collect() }
fn r(p: *const u32) -> String { if p.is_null() { return String::new(); } let mut n = 0; unsafe { while *p.add(n) != 0 { n += 1; } std::slice::from_raw_parts(p, n).iter().map(|&c| char::from_u32(c).unwrap_or('?')).collect() } }
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lib = unsafe { Library::new(&args[1]) }.unwrap();
    unsafe {
        let new: Symbol<NewFn> = lib.get(b"MediaInfo_New\0").unwrap();
        let delete: Symbol<DeleteFn> = lib.get(b"MediaInfo_Delete\0").unwrap();
        let open: Symbol<OpenFn> = lib.get(b"MediaInfo_Open\0").unwrap();
        let close: Symbol<CloseFn> = lib.get(b"MediaInfo_Close\0").unwrap();
        let inform: Symbol<InformFn> = lib.get(b"MediaInfo_Inform\0").unwrap();
        let get_i: Symbol<GetIFn> = lib.get(b"MediaInfo_GetI\0").unwrap();
        let option: Symbol<OptionFn> = lib.get(b"MediaInfo_Option\0").unwrap();
        let count: Symbol<CountGetFn> = lib.get(b"MediaInfo_Count_Get\0").unwrap();
        let mode = args[2].as_str();
        let h = new();
        option(h, w("setlocale_LC_CTYPE").as_ptr(), w("").as_ptr());
        option(h, w("FileTestContinuousFileNames").as_ptr(), w("0").as_ptr());
        if mode == "version" { println!("{}", r(option(h, w("Info_Version").as_ptr(), w("").as_ptr()))); return; }
        if mode == "opt" { println!("{}", r(option(h, w(&args[3]).as_ptr(), w(args.get(4).map(|s| s.as_str()).unwrap_or("")).as_ptr()))); return; }
        if mode == "params" { println!("{}", r(option(h, w("Info_Parameters").as_ptr(), w("").as_ptr()))); return; }
        if mode == "codecs" { println!("{}", r(option(h, w("Info_Codecs").as_ptr(), w("").as_ptr()))); return; }
        if mode == "inform" || mode == "complete" || mode == "xml" || mode == "json" {
            if mode == "complete" { option(h, w("Complete").as_ptr(), w("1").as_ptr()); }
            if mode == "xml" { option(h, w("Output").as_ptr(), w("XML").as_ptr()); }
            if mode == "json" { option(h, w("Output").as_ptr(), w("JSON").as_ptr()); }
            open(h, w(&args[3]).as_ptr());
            print!("{}", r(inform(h, 0)));
            close(h); delete(h); return;
        }
        // raw: every field of every stream: kind|index|param|name|text|measure|info
        open(h, w(&args[3]).as_ptr());
        let names = ["General","Video","Audio","Text","Other","Image","Menu"];
        for kind in 0..7 {
            let sc = count(h, kind, usize::MAX);
            for si in 0..sc {
                let fc = count(h, kind, si);
                println!("== {} #{} ({} fields)", names[kind], si, fc);
                for f in 0..fc {
                    let name = r(get_i(h, kind, si, f, 0));
                    let text = r(get_i(h, kind, si, f, 1));
                    let measure = r(get_i(h, kind, si, f, 2));
                    let options = r(get_i(h, kind, si, f, 3));
                    if text.is_empty() && mode != "schema" { continue; }
                    println!("{}\t{}\t{}\t{}\t{}", f, name, text.replace('\n', "\\n"), measure, options);
                }
            }
        }
        close(h); delete(h);
    }
}
