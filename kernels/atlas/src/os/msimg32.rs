//! msimg32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "MSIMG32.dll";
    missing AlphaBlend;
    missing GradientFill;
    missing TransparentBlt;
}
