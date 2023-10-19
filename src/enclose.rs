//! A little macro for cloning things before a closure.

/// Enclose arguments in a clone before a closure.
#[macro_export]
macro_rules! enclose {
    ( ($( $x:ident ),*) $y:expr ) => {
        {
            $(let $x = $x.clone();)*
            $y
        }
    };
}
