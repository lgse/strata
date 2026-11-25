// SPDX-License-Identifier: MIT

use rune::El;

pub(crate) fn on_return(key: impl Into<String>, button: El) -> El {
    taking_return(key, button).press_on_return()
}

pub(crate) fn taking_return(key: impl Into<String>, button: El) -> El {
    let key = key.into();
    #[cfg(test)]
    tests::taking_return(&key);
    button.key(key)
}

#[cfg(test)]
mod tests;
