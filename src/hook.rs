pub use crate::platform::{get_symbol, hook, write_memory};

pub unsafe fn hook_symbol(symbol: &str, func: *const ()) -> *const () {
	hook(get_symbol(symbol), func)
}
