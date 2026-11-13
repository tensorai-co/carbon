//! oleacc.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "OLEACC.dll";
    missing AccessibleChildren;
    missing AccessibleObjectFromWindow;
    missing CreateStdAccessibleObject;
    missing LresultFromObject;
    missing WindowFromAccessibleObject;
}
