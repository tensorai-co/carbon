//! ucrtbase.dll: the Universal C Runtime, which Windows 10 ships as part of the OS, so atlas has its own. Every export
//! is guest code in ntdll (`guest/src/crt.rs`, `stdio.rs`): the C runtime runs in the program. Programs reach it by
//! name or through its API sets (`api-ms-win-crt-*`), as MSVC's `/MD` and MinGW's UCRT programs do.

use exec::{FpRegs, Isa};
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "ucrtbase.dll";
    // The heap.
    guest malloc => AtlasCrtMalloc;
    guest calloc => AtlasCrtCalloc;
    guest realloc => AtlasCrtRealloc;
    guest free => AtlasCrtFree;
    guest _msize => AtlasCrtMsize;
    guest _set_new_mode => AtlasCrtSetNewMode;
    guest _callnewh => AtlasCrtIgnore;
    // Memory.
    guest memcpy => AtlasCrtMemcpy;
    guest memmove => AtlasCrtMemmove;
    guest memset => AtlasCrtMemset;
    guest memcmp => AtlasCrtMemcmp;
    guest memchr => AtlasCrtMemchr;
    guest memcpy_s => AtlasCrtMemcpyS;
    // Strings.
    guest strlen => AtlasCrtStrlen;
    guest strnlen => AtlasCrtStrnlen;
    guest strcmp => AtlasCrtStrcmp;
    guest strncmp => AtlasCrtStrncmp;
    guest _stricmp => AtlasCrtStricmp;
    guest _strnicmp => AtlasCrtStrnicmp;
    guest _strcmpi => AtlasCrtStricmp;
    guest strcpy => AtlasCrtStrcpy;
    guest strncpy => AtlasCrtStrncpy;
    guest strcat => AtlasCrtStrcat;
    guest strncat => AtlasCrtStrncat;
    guest strchr => AtlasCrtStrchr;
    guest strrchr => AtlasCrtStrrchr;
    guest strstr => AtlasCrtStrstr;
    guest _strdup => AtlasCrtStrdup;
    guest strspn => AtlasCrtStrspn;
    guest strcspn => AtlasCrtStrcspn;
    guest strpbrk => AtlasCrtStrpbrk;
    guest strtok => AtlasCrtStrtok;
    guest strtok_s => AtlasCrtStrtokS;
    guest strcpy_s => AtlasCrtStrcpyS;
    guest strcat_s => AtlasCrtStrcatS;
    guest strerror => AtlasCrtStrerror;
    guest wcslen => AtlasCrtWcslen;
    guest wcsnlen => AtlasCrtWcsnlen;
    guest wcscmp => AtlasCrtWcscmp;
    guest wcsncmp => AtlasCrtWcsncmp;
    guest _wcsicmp => AtlasCrtWcsicmp;
    guest _wcsnicmp => AtlasCrtWcsnicmp;
    guest wcscpy => AtlasCrtWcscpy;
    guest wcsncpy => AtlasCrtWcsncpy;
    guest wcscat => AtlasCrtWcscat;
    guest wcsncat => AtlasCrtWcsncat;
    guest wcschr => AtlasCrtWcschr;
    guest wcsrchr => AtlasCrtWcsrchr;
    guest wcsstr => AtlasCrtWcsstr;
    guest _wcsdup => AtlasCrtWcsdup;
    guest wcsspn => AtlasCrtWcsspn;
    guest wcscspn => AtlasCrtWcscspn;
    guest wcspbrk => AtlasCrtWcspbrk;
    // Characters.
    guest isalpha => AtlasCrtIsalpha;
    guest isupper => AtlasCrtIsupper;
    guest islower => AtlasCrtIslower;
    guest isdigit => AtlasCrtIsdigit;
    guest isxdigit => AtlasCrtIsxdigit;
    guest isspace => AtlasCrtIsspace;
    guest ispunct => AtlasCrtIspunct;
    guest iscntrl => AtlasCrtIscntrl;
    guest isblank => AtlasCrtIsblank;
    guest isalnum => AtlasCrtIsalnum;
    guest isprint => AtlasCrtIsprint;
    guest isgraph => AtlasCrtIsgraph;
    guest _isctype => AtlasCrtIsctype;
    guest __pctype_func => AtlasCrtPctype;
    guest tolower => AtlasCrtTolower;
    guest toupper => AtlasCrtToupper;
    guest towlower => AtlasCrtTowlower;
    guest towupper => AtlasCrtTowupper;
    guest iswalpha => AtlasCrtIswalpha;
    guest iswupper => AtlasCrtIswupper;
    guest iswlower => AtlasCrtIswlower;
    guest iswdigit => AtlasCrtIswdigit;
    guest iswxdigit => AtlasCrtIswxdigit;
    guest iswspace => AtlasCrtIswspace;
    guest iswpunct => AtlasCrtIswpunct;
    guest iswcntrl => AtlasCrtIswcntrl;
    guest iswalnum => AtlasCrtIswalnum;
    guest iswprint => AtlasCrtIswprint;
    guest iswctype => AtlasCrtIswctype;
    guest btowc => AtlasCrtBtowc;
    guest wctob => AtlasCrtWctob;
    // Numbers.
    guest atoi => AtlasCrtAtoi;
    guest atol => AtlasCrtAtoi;
    guest atoll => AtlasCrtAtoll;
    guest _wtoi => AtlasCrtWtoi;
    guest atof => AtlasCrtAtof;
    guest strtod => AtlasCrtStrtod;
    guest strtol => AtlasCrtStrtol;
    guest strtoul => AtlasCrtStrtoul;
    guest strtoll => AtlasCrtStrtoll;
    guest strtoull => AtlasCrtStrtoull;
    guest _strtoi64 => AtlasCrtStrtoll;
    guest _strtoui64 => AtlasCrtStrtoull;
    guest wcstol => AtlasCrtWcstol;
    guest wcstoul => AtlasCrtWcstoul;
    guest wcstoll => AtlasCrtWcstoll;
    guest wcstoull => AtlasCrtWcstoull;
    guest abs => AtlasCrtAbs;
    guest labs => AtlasCrtAbs;
    guest llabs => AtlasCrtLlabs;
    guest _abs64 => AtlasCrtLlabs;
    guest qsort => AtlasCrtQsort;
    guest qsort_s => AtlasCrtQsortS;
    guest bsearch => AtlasCrtBsearch;
    guest rand => AtlasCrtRand;
    guest srand => AtlasCrtSrand;
    // Startup and exit.
    guest _configure_narrow_argv => AtlasCrtConfigureNarrowArgv;
    guest _configure_wide_argv => AtlasCrtConfigureWideArgv;
    guest _initialize_narrow_environment => AtlasCrtInitializeNarrowEnvironment;
    guest _initialize_wide_environment => AtlasCrtInitializeWideEnvironment;
    guest _get_initial_narrow_environment => AtlasCrtGetInitialNarrowEnvironment;
    guest _get_initial_wide_environment => AtlasCrtGetInitialWideEnvironment;
    guest _get_narrow_winmain_command_line => AtlasCrtNarrowWinmainCommandLine;
    guest __p___argc => AtlasCrtArgc;
    guest __p___argv => AtlasCrtArgv;
    guest __p___wargv => AtlasCrtWargv;
    guest __p__environ => AtlasCrtEnviron;
    guest __p__wenviron => AtlasCrtWenviron;
    guest getenv => AtlasCrtGetenv;
    guest _initterm => AtlasCrtInitterm;
    guest _initterm_e => AtlasCrtInittermE;
    guest _initialize_onexit_table => AtlasCrtInitializeOnexitTable;
    guest _register_onexit_function => AtlasCrtRegisterOnexitFunction;
    guest _execute_onexit_table => AtlasCrtExecuteOnexitTable;
    guest _crt_atexit => AtlasCrtAtexit;
    guest _crt_at_quick_exit => AtlasCrtAtexit;
    guest atexit => AtlasCrtAtexit;
    guest _onexit => AtlasCrtOnexit;
    guest exit => AtlasCrtExit;
    guest _exit => AtlasCrtUnderscoreExit;
    guest _Exit => AtlasCrtUnderscoreExit;
    guest quick_exit => AtlasCrtUnderscoreExit;
    guest _cexit => AtlasCrtCexit;
    guest _c_exit => AtlasCrtCExitNothing;
    guest abort => AtlasCrtAbort;
    guest signal => AtlasCrtSignal;
    guest raise => AtlasCrtRaise;
    guest _set_app_type => AtlasCrtSetAppType;
    guest __p__fmode => AtlasCrtFmode;
    guest __p__commode => AtlasCrtCommode;
    guest _set_fmode => AtlasCrtSetFmode;
    guest _configthreadlocale => AtlasCrtConfigThreadLocale;
    guest __setusermatherr => AtlasCrtIgnore;
    guest _set_abort_behavior => AtlasCrtIgnore;
    guest _register_thread_local_exe_atexit_callback => AtlasCrtIgnore;
    guest _seh_filter_exe => AtlasCrtIgnore;
    guest _set_invalid_parameter_handler => AtlasCrtSetInvalidParameterHandler;
    guest _set_thread_local_invalid_parameter_handler => AtlasCrtSetInvalidParameterHandler;
    guest _invalid_parameter_noinfo => AtlasCrtInvalidParameterNoinfo;
    guest _invalid_parameter_noinfo_noreturn => AtlasCrtInvalidParameterNoinfoNoreturn;
    guest _errno => AtlasCrtErrno;
    guest setlocale => AtlasCrtSetlocale;
    guest ___lc_codepage_func => AtlasCrtLcCodepage;
    guest ___mb_cur_max_func => AtlasCrtMbCurMax;
    guest _time64 => AtlasCrtTime64;
    guest time => AtlasCrtTime64;
    guest clock => AtlasCrtClock;
    guest __daylight => AtlasCrtDaylight;
    guest __timezone => AtlasCrtTimezone;
    guest __tzname => AtlasCrtTzname;
    guest _tzset => AtlasCrtIgnore;
    guest __C_specific_handler;
    // stdio.
    guest __acrt_iob_func => AtlasCrtIob;
    guest __stdio_common_vfprintf => AtlasCrtVfprintf;
    guest __stdio_common_vfwprintf => AtlasCrtVfwprintf;
    guest __stdio_common_vsprintf => AtlasCrtVsprintf;
    guest __stdio_common_vswprintf => AtlasCrtVswprintf;
    guest __stdio_common_vsprintf_s => AtlasCrtVsprintfS;
    guest __stdio_common_vsnprintf_s => AtlasCrtVsnprintfS;
    guest fopen => AtlasCrtFopen;
    guest fclose => AtlasCrtFclose;
    guest fflush => AtlasCrtFflush;
    guest setvbuf => AtlasCrtSetvbuf;
    guest fwrite => AtlasCrtFwrite;
    guest fread => AtlasCrtFread;
    guest fputs => AtlasCrtFputs;
    guest puts => AtlasCrtPuts;
    guest fputc => AtlasCrtFputc;
    guest putc => AtlasCrtFputc;
    guest putchar => AtlasCrtPutchar;
    guest fgetc => AtlasCrtFgetc;
    guest getc => AtlasCrtFgetc;
    guest getchar => AtlasCrtGetchar;
    guest ungetc => AtlasCrtUngetc;
    guest fgets => AtlasCrtFgets;
    guest feof => AtlasCrtFeof;
    guest ferror => AtlasCrtFerror;
    guest clearerr => AtlasCrtClearerr;
    guest fseek => AtlasCrtFseek;
    guest _fseeki64 => AtlasCrtFseeki64;
    guest ftell => AtlasCrtFtell;
    guest _ftelli64 => AtlasCrtFtelli64;
    guest rewind => AtlasCrtRewind;
    guest _fileno => AtlasCrtFileno;
    guest _setmode => AtlasCrtSetmode;
    // Extended UCRT: locale, aligned alloc, time, threads, filesystem, POSIX fds, setjmp/longjmp.
    guest _Getdays => AtlasCrtGetdays;
    guest _Getmonths => AtlasCrtGetmonths;
    guest _Gettnames => AtlasCrtGettnames;
    guest _Strftime => AtlasCrtStrftimeL;
    guest _W_Getdays => AtlasCrtWGetdays;
    guest _W_Getmonths => AtlasCrtWGetmonths;
    guest _W_Gettnames => AtlasCrtWGettnames;
    guest _Wcsftime => AtlasCrtWcsftime;
    missing __CxxFrameHandler3;
    guest ___lc_collate_cp_func => AtlasCrtLcCollateCp;
    guest ___lc_locale_name_func => AtlasCrtLcLocaleName;
    guest __stdio_common_vswprintf_s => AtlasCrtVswprintfS;
    guest __strncnt => AtlasCrtStrncnt;
    guest _aligned_free => AtlasCrtAlignedFree;
    guest _aligned_offset_malloc => AtlasCrtAlignedOffsetMalloc;
    guest _beginthreadex => AtlasCrtBeginthreadex;
    guest _endthreadex => AtlasCrtEndthreadex;
    guest _fsopen => AtlasCrtFsopen;
    guest _get_stream_buffer_pointers => AtlasCrtGetStreamBufferPointers;
    guest _invoke_watson => AtlasCrtInvokeWatson;
    guest _lock_file => AtlasCrtLockFile;
    guest _lock_locales => AtlasCrtLockLocales;
    missing _o___std_type_info_destroy_list;
    guest _o___stdio_common_vswprintf_s => AtlasCrtVswprintfS;
    guest _o__aligned_free => AtlasCrtAlignedFree;
    guest _o__aligned_malloc => AtlasCrtAlignedMalloc;
    guest _o__cexit => AtlasCrtCexit;
    guest _o__configure_narrow_argv => AtlasCrtConfigureNarrowArgv;
    guest _o__execute_onexit_table => AtlasCrtExecuteOnexitTable;
    guest _o__initialize_narrow_environment => AtlasCrtInitializeNarrowEnvironment;
    guest _o__initialize_onexit_table => AtlasCrtInitializeOnexitTable;
    guest _o__purecall => AtlasCrtPurecall;
    guest _o__seh_filter_dll => AtlasCrtIgnore;
    guest _o__strlwr_s => AtlasCrtStrlwrS;
    guest _o__wcsicmp => AtlasCrtWcsicmp;
    guest _o__wcsnicmp => AtlasCrtWcsnicmp;
    guest _o__wsplitpath_s => AtlasCrtWsplitpathS;
    guest _o_free => AtlasCrtFree;
    guest _o_malloc => AtlasCrtMalloc;
    guest _o_rand => AtlasCrtRand;
    guest _o_srand => AtlasCrtSrand;
    guest _o_towlower => AtlasCrtTowlower;
    guest _seh_filter_dll => AtlasCrtIgnore;
    guest _set_new_handler => AtlasCrtSetNewHandler;
    guest _unlock_file => AtlasCrtUnlockFile;
    guest _unlock_locales => AtlasCrtUnlockLocales;
    guest _wchdir => AtlasCrtWchdir;
    guest _wfsopen => AtlasCrtWfsopen;
    guest _wgetcwd => AtlasCrtWgetcwd;
    guest _wremove => AtlasCrtWremove;
    guest _wrename => AtlasCrtWrename;
    guest _wrmdir => AtlasCrtWrmdir;
    // Math (host IEEE-754).
    host acos(1) => math_acos;
    host acosf(1) => math_acosf;
    host acosh(1) => math_acosh;
    host asin(1) => math_asin;
    host asinf(1) => math_asinf;
    host asinh(1) => math_asinh;
    host atan(1) => math_atan;
    host atan2(2) => math_atan2;
    host atan2f(2) => math_atan2f;
    host atanf(1) => math_atanf;
    host atanh(1) => math_atanh;
    host cbrtf(1) => math_cbrtf;
    host ceil(1) => math_ceil;
    host ceilf(1) => math_ceilf;
    host copysign(2) => math_copysign;
    host cos(1) => math_cos;
    host cosf(1) => math_cosf;
    host cosh(1) => math_cosh;
    host erfc(1) => math_erfc;
    host exp(1) => math_exp;
    host exp2f(1) => math_exp2f;
    host expf(1) => math_expf;
    host floor(1) => math_floor;
    host floorf(1) => math_floorf;
    host fmod(2) => math_fmod;
    host fmodf(2) => math_fmodf;
    host frexp(2) => math_frexp;
    host _hypot(2) => math_hypot;
    host ldexp(2) => math_ldexp;
    host log(1) => math_log;
    host log10(1) => math_log10;
    host log10f(1) => math_log10f;
    host log2(1) => math_log2;
    host log2f(1) => math_log2f;
    host logb(1) => math_logb;
    host logbf(1) => math_logbf;
    host logf(1) => math_logf;
    host lround(1) => math_lround;
    host lroundf(1) => math_lroundf;
    host modf(2) => math_modf;
    host modff(2) => math_modff;
    host nextafter(2) => math_nextafter;
    host nextafterf(2) => math_nextafterf;
    host pow(2) => math_pow;
    host powf(2) => math_powf;
    host round(1) => math_round;
    host roundf(1) => math_roundf;
    host sin(1) => math_sin;
    host sinf(1) => math_sinf;
    host sinh(1) => math_sinh;
    host sinhf(1) => math_sinhf;
    host sqrt(1) => math_sqrt;
    host sqrtf(1) => math_sqrtf;
    host tan(1) => math_tan;
    host tanf(1) => math_tanf;
    host tanh(1) => math_tanh;
    host trunc(1) => math_trunc;
    host truncf(1) => math_truncf;
    guest fgetpos => AtlasCrtFgetpos;
    guest fgetwc => AtlasCrtFgetwc;
    guest fputwc => AtlasCrtFputwc;
    guest fsetpos => AtlasCrtFsetpos;
    guest localeconv => AtlasCrtLocaleconv;
    host rand_s(1) => crt_rand_s;
    guest strtof => AtlasCrtStrtof;
    guest terminate => AtlasCrtTerminate;
    guest ungetwc => AtlasCrtUngetwc;
    guest wcscpy_s => AtlasCrtWcscpyS;
    missing __DestructExceptionObject;
    guest __intrinsic_setjmp => AtlasCrtSetjmp;
    guest __stdio_common_vfscanf => AtlasCrtVfscanf;
    guest __stdio_common_vsnwprintf_s => AtlasCrtVsnwprintfS;
    guest __stdio_common_vsscanf => AtlasCrtVsscanf;
    guest __sys_nerr => AtlasCrtSysNerr;
    guest _aligned_malloc => AtlasCrtAlignedMalloc;
    guest _assert => AtlasCrtAssert;
    guest _chdir => AtlasCrtChdir;
    guest _close => AtlasCrtClose;
    guest _create_locale => AtlasCrtCreateLocale;
    guest _difftime64 => AtlasCrtDifftime64;
    guest _dup => AtlasCrtDup;
    guest _dup2 => AtlasCrtDup2;
    guest _fdopen => AtlasCrtFdopen;
    guest _findclose => AtlasCrtFindclose;
    guest _findfirst64i32 => AtlasCrtFindfirst64i32;
    guest _findnext64i32 => AtlasCrtFindnext64i32;
    guest _free_locale => AtlasCrtFreeLocale;
    guest _fstat64 => AtlasCrtFstat64;
    guest _fstat64i32 => AtlasCrtFstat64i32;
    guest _fullpath => AtlasCrtFullpath;
    guest _get_osfhandle => AtlasCrtGetOsfhandle;
    guest _getcwd => AtlasCrtGetcwd;
    guest _gmtime64 => AtlasCrtGmtime64;
    guest _gmtime64_s => AtlasCrtGmtime64S;
    guest _i64toa => AtlasCrtI64toa;
    guest _isatty => AtlasCrtIsatty;
    guest _iswalpha_l => AtlasCrtIswalphaL;
    guest _iswcntrl_l => AtlasCrtIswcntrlL;
    guest _iswdigit_l => AtlasCrtIswdigitL;
    guest _iswlower_l => AtlasCrtIswlowerL;
    guest _iswprint_l => AtlasCrtIswprintL;
    guest _iswpunct_l => AtlasCrtIswpunctL;
    guest _iswspace_l => AtlasCrtIswspaceL;
    guest _iswupper_l => AtlasCrtIswupperL;
    guest _iswxdigit_l => AtlasCrtIswxdigitL;
    guest _itoa => AtlasCrtItoa;
    guest _localtime64 => AtlasCrtGmtime64;
    guest _localtime64_s => AtlasCrtGmtime64S;
    guest _lseeki64 => AtlasCrtLseeki64;
    guest _ltoa => AtlasCrtItoa;
    guest _mbtowc_l => AtlasCrtMbtowcL;
    missing _mktemp_s;
    guest _mktime64 => AtlasCrtMktime64;
    guest _open => AtlasCrtOpen;
    missing _pclose;
    missing _pipe;
    missing _popen;
    guest _putenv_s => AtlasCrtPutenvS;
    guest _read => AtlasCrtRead;
    guest _rmdir => AtlasCrtRmdir;
    guest _set_errno => AtlasCrtSetErrno;
    guest _sopen_s => AtlasCrtSopenS;
    guest _splitpath_s => AtlasCrtSplitpathS;
    guest _stat64 => AtlasCrtStat64;
    guest _stat64i32 => AtlasCrtStat64i32;
    guest _strcoll_l => AtlasCrtStrcollL;
    guest _strftime_l => AtlasCrtStrftimeL;
    guest _strlwr => AtlasCrtStrlwr;
    guest _strrev => AtlasCrtStrrev;
    guest _strtod_l => AtlasCrtStrtodL;
    guest _strupr => AtlasCrtStrupr;
    guest _strxfrm_l => AtlasCrtStrxfrmL;
    guest _tolower_l => AtlasCrtTolowerL;
    guest _toupper_l => AtlasCrtToupperL;
    guest _towlower_l => AtlasCrtTowlowerL;
    guest _towupper_l => AtlasCrtTowupperL;
    guest _ui64toa => AtlasCrtUi64toa;
    guest _ultoa => AtlasCrtUltoa;
    guest _unlink => AtlasCrtUnlink;
    guest _wassert => AtlasCrtWassert;
    guest _wcscoll_l => AtlasCrtWcscollL;
    guest _wcsxfrm_l => AtlasCrtWcsxfrmL;
    guest _wfopen => AtlasCrtWfopen;
    guest _wgetenv => AtlasCrtWgetenv;
    guest _wopen => AtlasCrtWopen;
    guest _write => AtlasCrtWrite;
    guest freopen => AtlasCrtFreopen;
    guest freopen_s => AtlasCrtFreopenS;
    guest longjmp => AtlasCrtLongjmp;
    guest mbrlen => AtlasCrtMbrlen;
    guest mbrtowc => AtlasCrtMbrtowc;
    guest mbsrtowcs => AtlasCrtMbsrtowcs;
    guest mbstowcs => AtlasCrtMbstowcs;
    guest mbstowcs_s => AtlasCrtMbstowcsS;
    guest remove => AtlasCrtUnlink;
    guest rename => AtlasCrtRename;
    guest setbuf => AtlasCrtSetbuf;
    guest strerror_s => AtlasCrtStrerrorS;
    guest strftime => AtlasCrtStrftime;
    guest strncpy_s => AtlasCrtStrncpyS;
    missing system;
    missing tmpfile;
    missing tmpnam;
    guest wcrtomb => AtlasCrtWcrtomb;
    guest wcrtomb_s => AtlasCrtWcrtombS;
    guest wcsncpy_s => AtlasCrtWcsncpyS;
    guest wcstod => AtlasCrtWcstod;
    guest wcstombs => AtlasCrtWcstombs;
    guest wcstombs_s => AtlasCrtWcstombsS;
}

fn crt_rand_s(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || !task.random_into(cx, a[0], 4) {
        return Reply::Value(22);
    }
    Reply::Value(0)
}

// ---- Floating-point ABI helpers and C99 math ----

fn f64_arg(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], index: usize, float: usize) -> f64 {
    let fp = cx.turn(|t| t.thread().fp());
    match (task.isa, fp) {
        (Isa::X86_64, FpRegs::X86(f)) if index < 4 => {
            let at = 160 + 16 * index;
            f64::from_le_bytes(f.fxsave[at..at + 8].try_into().unwrap())
        }
        (Isa::Arm64, FpRegs::Arm64(f)) => f64::from_bits(f.v[float] as u64),
        _ => f64::from_bits(a.get(index).copied().unwrap_or(0)),
    }
}

fn return_f64(task: &mut Task, cx: &mut Cx<'_>, v: f64) -> Reply {
    let isa = task.isa;
    cx.turn(|t| {
        let mut fp = t.thread().fp();
        match (isa, &mut fp) {
            (Isa::X86_64, FpRegs::X86(f)) => f.fxsave[160..168].copy_from_slice(&v.to_le_bytes()),
            (Isa::Arm64, FpRegs::Arm64(f)) => f.v[0] = u128::from(v.to_bits()),
            _ => {}
        }
        t.thread().set_fp(&fp);
    });
    Reply::Value(v.to_bits())
}

fn return_f32(task: &mut Task, cx: &mut Cx<'_>, v: f32) -> Reply {
    let isa = task.isa;
    cx.turn(|t| {
        let mut fp = t.thread().fp();
        match (isa, &mut fp) {
            (Isa::X86_64, FpRegs::X86(f)) => f.fxsave[160..164].copy_from_slice(&v.to_le_bytes()),
            (Isa::Arm64, FpRegs::Arm64(f)) => f.v[0] = u128::from(v.to_bits()),
            _ => {}
        }
        t.thread().set_fp(&fp);
    });
    Reply::Value(u64::from(v.to_bits()))
}

macro_rules! math_f64_1 {
    ($name:ident, $f:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            let x = f64_arg(task, cx, a, 0, 0);
            return_f64(task, cx, ($f)(x))
        }
    };
}

macro_rules! math_f64_2 {
    ($name:ident, $f:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            let x = f64_arg(task, cx, a, 0, 0);
            let y = f64_arg(task, cx, a, 1, 1);
            return_f64(task, cx, ($f)(x, y))
        }
    };
}

macro_rules! math_f32_1 {
    ($name:ident, $f:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            let x = task.f32_arg(cx, a, 0, 0);
            return_f32(task, cx, ($f)(x))
        }
    };
}

macro_rules! math_f32_2 {
    ($name:ident, $f:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            let x = task.f32_arg(cx, a, 0, 0);
            let y = task.f32_arg(cx, a, 1, 1);
            return_f32(task, cx, ($f)(x, y))
        }
    };
}

math_f64_1!(math_acos, f64::acos);
math_f32_1!(math_acosf, f32::acos);
math_f64_1!(math_acosh, f64::acosh);
math_f64_1!(math_asin, f64::asin);
math_f32_1!(math_asinf, f32::asin);
math_f64_1!(math_asinh, f64::asinh);
math_f64_1!(math_atan, f64::atan);
math_f64_2!(math_atan2, f64::atan2);
math_f32_2!(math_atan2f, f32::atan2);
math_f32_1!(math_atanf, f32::atan);
math_f64_1!(math_atanh, f64::atanh);
math_f32_1!(math_cbrtf, f32::cbrt);
math_f64_1!(math_ceil, f64::ceil);
math_f32_1!(math_ceilf, f32::ceil);
math_f64_2!(math_copysign, f64::copysign);
math_f64_1!(math_cos, f64::cos);
math_f32_1!(math_cosf, f32::cos);
math_f64_1!(math_cosh, f64::cosh);
math_f64_1!(math_exp, f64::exp);
math_f32_1!(math_exp2f, f32::exp2);
math_f32_1!(math_expf, f32::exp);
math_f64_1!(math_floor, f64::floor);
math_f32_1!(math_floorf, f32::floor);
math_f64_2!(math_fmod, |x: f64, y: f64| x % y);
math_f32_2!(math_fmodf, |x: f32, y: f32| x % y);
math_f64_2!(math_hypot, f64::hypot);
math_f64_1!(math_log, f64::ln);
math_f64_1!(math_log10, f64::log10);
math_f32_1!(math_log10f, f32::log10);
math_f64_1!(math_log2, f64::log2);
math_f32_1!(math_log2f, f32::log2);
math_f32_1!(math_logf, f32::ln);
math_f64_2!(math_pow, f64::powf);
math_f32_2!(math_powf, f32::powf);
math_f64_1!(math_round, f64::round);
math_f32_1!(math_roundf, f32::round);
math_f64_1!(math_sin, f64::sin);
math_f32_1!(math_sinf, f32::sin);
math_f64_1!(math_sinh, f64::sinh);
math_f32_1!(math_sinhf, f32::sinh);
math_f64_1!(math_sqrt, f64::sqrt);
math_f32_1!(math_sqrtf, f32::sqrt);
math_f64_1!(math_tan, f64::tan);
math_f32_1!(math_tanf, f32::tan);
math_f64_1!(math_tanh, f64::tanh);
math_f64_1!(math_trunc, f64::trunc);
math_f32_1!(math_truncf, f32::trunc);

fn math_logb(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0).abs();
    let r = if x == 0.0 {
        f64::NEG_INFINITY
    } else if x.is_infinite() {
        f64::INFINITY
    } else if x.is_nan() {
        x
    } else {
        x.log2().floor()
    };
    return_f64(task, cx, r)
}

fn math_logbf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.f32_arg(cx, a, 0, 0).abs();
    let r = if x == 0.0 {
        f32::NEG_INFINITY
    } else if x.is_infinite() {
        f32::INFINITY
    } else if x.is_nan() {
        x
    } else {
        x.log2().floor()
    };
    return_f32(task, cx, r)
}

fn math_lround(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0).round();
    Reply::Value((x as i32) as u32 as u64)
}

fn math_lroundf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.f32_arg(cx, a, 0, 0).round();
    Reply::Value((x as i32) as u32 as u64)
}

fn math_modf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0);
    let iptr = if task.isa == Isa::Arm64 { a[0] } else { a[1] };
    let int = x.trunc();
    let frac = if x.is_infinite() { 0.0f64.copysign(x) } else { x - int };
    if iptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), iptr, &int.to_le_bytes()));
    }
    return_f64(task, cx, frac)
}

fn math_modff(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.f32_arg(cx, a, 0, 0);
    let iptr = if task.isa == Isa::Arm64 { a[0] } else { a[1] };
    let int = x.trunc();
    let frac = if x.is_infinite() { 0.0f32.copysign(x) } else { x - int };
    if iptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), iptr, &int.to_le_bytes()));
    }
    return_f32(task, cx, frac)
}

fn math_frexp(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0);
    let exp_ptr = if task.isa == Isa::Arm64 { a[0] } else { a[1] };
    let (frac, exp): (f64, i32) = if x == 0.0 || !x.is_finite() {
        (x, 0)
    } else {
        let bits = x.to_bits();
        let sign = bits & (1u64 << 63);
        let biased = ((bits >> 52) & 0x7ff) as i32;
        if biased == 0 {
            let scaled = x * (1u64 << 54) as f64;
            let sb = scaled.to_bits();
            let e = ((sb >> 52) & 0x7ff) as i32 - 1022 - 54;
            let m = f64::from_bits(sign | (sb & 0x000f_ffff_ffff_ffff) | (1022u64 << 52));
            (m, e)
        } else {
            let e = biased - 1022;
            let m = f64::from_bits(sign | (bits & 0x000f_ffff_ffff_ffff) | (1022u64 << 52));
            (m, e)
        }
    };
    if exp_ptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), exp_ptr, &exp.to_le_bytes()));
    }
    return_f64(task, cx, frac)
}

fn math_ldexp(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0);
    let exp = (if task.isa == Isa::Arm64 { a[0] } else { a[1] }) as u32 as i32;
    let r = if x == 0.0 || !x.is_finite() || exp == 0 {
        x
    } else {
        x * (2.0f64).powi(exp)
    };
    return_f64(task, cx, r)
}

fn math_nextafter(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0);
    let y = f64_arg(task, cx, a, 1, 1);
    let r = if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        y
    } else if y > x {
        x.next_up()
    } else {
        x.next_down()
    };
    return_f64(task, cx, r)
}

fn math_nextafterf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.f32_arg(cx, a, 0, 0);
    let y = task.f32_arg(cx, a, 1, 1);
    let r = if x.is_nan() || y.is_nan() {
        f32::NAN
    } else if x == y {
        y
    } else if y > x {
        x.next_up()
    } else {
        x.next_down()
    };
    return_f32(task, cx, r)
}

fn math_erfc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = f64_arg(task, cx, a, 0, 0);
    let ax = x.abs();
    let t = 1.0 / (1.0 + 0.5 * ax);
    let tau = t
        * (-ax * ax - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886807
                                + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))))
            .exp();
    let r = if x >= 0.0 { tau } else { 2.0 - tau };
    return_f64(task, cx, r)
}
