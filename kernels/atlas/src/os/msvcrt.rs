//! msvcrt.dll: legacy C runtime, forwarded to atlas's guest CRT and host math helpers.

use kernels_core::thread::Cx;

use super::ucrtbase::{
    crt_isascii, crt_memicmp, crt_neg1, crt_wcscat_s, crt_wcsncat_s, crt_wfullpath, crt_wmakepath_s, crt_zero,
    math_acos, math_asin, math_atan, math_atan2, math_ceil, math_cos, math_cosh, math_exp, math_finite, math_floor,
    math_floorf, math_fmod, math_fpclass, math_isnan, math_log, math_modf, math_pow, math_sin, math_sinh, math_sqrt,
    math_tan, math_tanh,
};
use super::{Reply, dll};
use crate::process::Task;

dll! {
    "msvcrt.dll";
    host "??1type_info@@UEAA@XZ"(1) => crt_zero;
    guest "?terminate@@YAXXZ" => AtlasCrtTerminate;
    missing _CxxThrowException;
    host _XcptFilter(2) => crt_zero;
    guest __C_specific_handler;
    missing __CxxFrameHandler3;
    host __dllonexit(3) => dllonexit;
    host __isascii(1) => crt_isascii;
    host __unDName(7) => crt_zero;
    guest _amsg_exit => AtlasCrtAbort;
    guest _atoi64 => AtlasCrtAtoll;
    guest _callnewh => AtlasCrtIgnore;
    host _chsize(2) => crt_zero;
    host _chsize_s(2) => crt_zero;
    host _clearfp(0) => crt_zero;
    guest _close => AtlasCrtClose;
    host _controlfp(2) => crt_zero;
    guest _errno => AtlasCrtErrno;
    host _filelengthi64(1) => crt_neg1;
    guest _fileno => AtlasCrtFileno;
    host _finite(1) => math_finite;
    host _fpclass(1) => math_fpclass;
    guest _get_osfhandle => AtlasCrtGetOsfhandle;
    guest _initterm => AtlasCrtInitterm;
    host _isnan(1) => math_isnan;
    host _lock(1) => crt_zero;
    guest _lseeki64 => AtlasCrtLseeki64;
    guest _mbscmp => AtlasCrtStrcmp;
    guest _mbstrlen => AtlasCrtStrlen;
    host _memicmp(3) => crt_memicmp;
    guest _onexit => AtlasCrtOnexit;
    host _open_osfhandle(2) => crt_neg1;
    guest _purecall => AtlasCrtPurecall;
    guest _read => AtlasCrtRead;
    guest _snwprintf_s => AtlasCrtMsvcrtSnwprintfS;
    guest _strdup => AtlasCrtStrdup;
    guest _stricmp => AtlasCrtStricmp;
    guest _strnicmp => AtlasCrtStrnicmp;
    guest _strtoui64 => AtlasCrtStrtoull;
    guest _time64 => AtlasCrtTime64;
    host _unlock(1) => crt_zero;
    guest _vsnprintf => AtlasCrtMsvcrtVsnprintf;
    guest _vsnwprintf => AtlasCrtMsvcrtVsnwprintf;
    guest _wcsdup => AtlasCrtWcsdup;
    guest _wcsicmp => AtlasCrtWcsicmp;
    guest _wcsnicmp => AtlasCrtWcsnicmp;
    guest _wfsopen => AtlasCrtWfsopen;
    host _wfullpath(3) => crt_wfullpath;
    guest _wgetenv => AtlasCrtWgetenv;
    host _wmakepath_s(6) => crt_wmakepath_s;
    guest _write => AtlasCrtWrite;
    host _wsopen(4) => crt_neg1;
    guest _wsplitpath_s => AtlasCrtWsplitpathS;
    host acos(1) => math_acos;
    host asin(1) => math_asin;
    host atan(1) => math_atan;
    host atan2(2) => math_atan2;
    guest atof => AtlasCrtAtof;
    guest atoi => AtlasCrtAtoi;
    guest bsearch => AtlasCrtBsearch;
    host ceil(1) => math_ceil;
    host cos(1) => math_cos;
    host cosh(1) => math_cosh;
    host exp(1) => math_exp;
    guest fclose => AtlasCrtFclose;
    host floor(1) => math_floor;
    host floorf(1) => math_floorf;
    host fmod(2) => math_fmod;
    guest fread => AtlasCrtFread;
    guest free => AtlasCrtFree;
    guest fseek => AtlasCrtFseek;
    guest ftell => AtlasCrtFtell;
    guest getenv => AtlasCrtGetenv;
    guest isalnum => AtlasCrtIsalnum;
    guest isalpha => AtlasCrtIsalpha;
    guest isdigit => AtlasCrtIsdigit;
    guest isspace => AtlasCrtIsspace;
    guest iswdigit => AtlasCrtIswdigit;
    guest isxdigit => AtlasCrtIsxdigit;
    host log(1) => math_log;
    guest malloc => AtlasCrtMalloc;
    guest memcmp => AtlasCrtMemcmp;
    guest memcpy => AtlasCrtMemcpy;
    guest memcpy_s => AtlasCrtMemcpyS;
    guest memmove => AtlasCrtMemmove;
    guest memset => AtlasCrtMemset;
    host modf(2) => math_modf;
    host pow(2) => math_pow;
    guest qsort => AtlasCrtQsort;
    guest setlocale => AtlasCrtSetlocale;
    host sin(1) => math_sin;
    host sinh(1) => math_sinh;
    guest sprintf_s => AtlasCrtMsvcrtSprintfS;
    host sqrt(1) => math_sqrt;
    guest sscanf => AtlasCrtMsvcrtSscanf;
    guest sscanf_s => AtlasCrtMsvcrtSscanf;
    guest strcat_s => AtlasCrtStrcatS;
    guest strchr => AtlasCrtStrchr;
    guest strcmp => AtlasCrtStrcmp;
    guest strcpy_s => AtlasCrtStrcpyS;
    guest strncmp => AtlasCrtStrncmp;
    guest strncpy_s => AtlasCrtStrncpyS;
    guest strnlen => AtlasCrtStrnlen;
    guest strrchr => AtlasCrtStrrchr;
    guest strstr => AtlasCrtStrstr;
    guest strtod => AtlasCrtStrtod;
    guest strtoul => AtlasCrtStrtoul;
    guest swprintf_s => AtlasCrtMsvcrtSwprintfS;
    host tan(1) => math_tan;
    host tanh(1) => math_tanh;
    guest tolower => AtlasCrtTolower;
    guest toupper => AtlasCrtToupper;
    guest towlower => AtlasCrtTowlower;
    guest vsprintf_s => AtlasCrtMsvcrtVsprintfS;
    host wcscat_s(3) => crt_wcscat_s;
    guest wcschr => AtlasCrtWcschr;
    guest wcscpy_s => AtlasCrtWcscpyS;
    host wcsncat_s(4) => crt_wcsncat_s;
    guest wcsncmp => AtlasCrtWcsncmp;
    guest wcsncpy_s => AtlasCrtWcsncpyS;
    guest wcsrchr => AtlasCrtWcsrchr;
    guest wcstol => AtlasCrtWcstol;
    guest wcstoul => AtlasCrtWcstoul;
}

fn dllonexit(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0])
}
