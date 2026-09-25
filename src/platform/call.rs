//! Calls from Rust into the game's code, through shim/call.c, which aligns the
//! stack to 16 bytes as the game's Linux gcc code expects.
//!
//! `call_game(f, (a, b, c))` works for any cdecl signature: arguments are
//! passed as 32-bit stack words and the return type picks the shim variant.

use std::ffi::c_void;

extern "C" {
	fn wal_call_int(f: *const c_void, count: u32, args: *const u32) -> u64;
	fn wal_call_float(f: *const c_void, count: u32, args: *const u32) -> f64;
}

const MAX_WORDS: usize = 16;

pub struct Words {
	words: [u32; MAX_WORDS],
	len: usize,
}

impl Words {
	fn push(&mut self, word: u32) {
		assert!(self.len < MAX_WORDS, "call_game: too many arguments");
		self.words[self.len] = word;
		self.len += 1;
	}
}

pub trait Arg {
	fn push_to(self, words: &mut Words);
}

macro_rules! word_arg {
	($($t:ty),*) => {$(
		impl Arg for $t {
			fn push_to(self, words: &mut Words) {
				words.push(self as u32);
			}
		}
	)*};
}
word_arg!(i8, u8, i16, u16, i32, u32, isize, usize);

impl Arg for bool {
	fn push_to(self, words: &mut Words) {
		words.push(self as u32);
	}
}

impl Arg for f32 {
	fn push_to(self, words: &mut Words) {
		words.push(self.to_bits());
	}
}

impl Arg for f64 {
	fn push_to(self, words: &mut Words) {
		let bits = self.to_bits();
		words.push(bits as u32);
		words.push((bits >> 32) as u32);
	}
}

impl<T> Arg for *const T {
	fn push_to(self, words: &mut Words) {
		words.push(self as usize as u32);
	}
}

impl<T> Arg for *mut T {
	fn push_to(self, words: &mut Words) {
		words.push(self as usize as u32);
	}
}

pub trait Args {
	fn words(self) -> Words;
}

macro_rules! tuple_args {
	($($name:ident)*) => {
		impl<$($name: Arg),*> Args for ($($name,)*) {
			#[allow(non_snake_case, unused_mut)]
			fn words(self) -> Words {
				let mut words = Words { words: [0; MAX_WORDS], len: 0 };
				let ($($name,)*) = self;
				$($name.push_to(&mut words);)*
				words
			}
		}
	};
}
tuple_args!();
tuple_args!(A);
tuple_args!(A B);
tuple_args!(A B C);
tuple_args!(A B C D);
tuple_args!(A B C D E);
tuple_args!(A B C D E F);
tuple_args!(A B C D E F G);
tuple_args!(A B C D E F G H);
tuple_args!(A B C D E F G H I);
tuple_args!(A B C D E F G H I J);

pub trait Ret {
	const FLOAT: bool = false;
	fn from_int(_value: u64) -> Self;
	fn from_float(_value: f64) -> Self
	where
		Self: Sized,
	{
		unreachable!()
	}
}

impl Ret for () {
	fn from_int(_: u64) {}
}

macro_rules! int_ret {
	($($t:ty),*) => {$(
		impl Ret for $t {
			fn from_int(value: u64) -> Self {
				value as $t
			}
		}
	)*};
}
int_ret!(i8, u8, i16, u16, i32, u32, i64, u64, isize, usize);

impl Ret for bool {
	// gcc returns bool in al; the rest of eax is undefined.
	fn from_int(value: u64) -> Self {
		value as u8 != 0
	}
}

impl<T> Ret for *const T {
	fn from_int(value: u64) -> Self {
		value as u32 as usize as *const T
	}
}

impl<T> Ret for *mut T {
	fn from_int(value: u64) -> Self {
		value as u32 as usize as *mut T
	}
}

impl Ret for f32 {
	const FLOAT: bool = true;
	fn from_int(_: u64) -> Self {
		unreachable!()
	}
	fn from_float(value: f64) -> Self {
		value as f32
	}
}

impl Ret for f64 {
	const FLOAT: bool = true;
	fn from_int(_: u64) -> Self {
		unreachable!()
	}
	fn from_float(value: f64) -> Self {
		value
	}
}

/// Calls the cdecl function `f` in game code. The caller picks the return
/// type, which must match the function's: a float or double result is read
/// from the x87 stack, anything else from eax:edx.
pub unsafe fn call_game<A: Args, R: Ret>(f: *const (), args: A) -> R {
	assert!(!f.is_null(), "call_game: null function pointer");
	let words = args.words();
	let f = f as *const c_void;
	if R::FLOAT {
		R::from_float(wal_call_float(f, words.len as u32, words.words.as_ptr()))
	} else {
		R::from_int(wal_call_int(f, words.len as u32, words.words.as_ptr()))
	}
}

#[cfg(test)]
mod tests {
	use super::call_game;
	use std::ffi::c_void;

	extern "C" fn sum(a: i32, b: u8, c: i16, d: u32) -> i32 {
		a + b as i32 + c as i32 + d as i32
	}
	extern "C" fn mix(a: f32, b: f64, c: i32, d: *const c_void) -> f64 {
		a as f64 * 10.0 + b + c as f64 + d as usize as f64
	}
	extern "C" fn half(a: f32) -> f32 {
		a / 2.0
	}
	extern "C" fn negate(a: bool) -> bool {
		!a
	}
	extern "C" fn wide(a: u32) -> u64 {
		(a as u64) << 32 | 7
	}
	extern "C" fn many(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: f64) -> i32 {
		a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h as i32
	}
	static mut WRITTEN: i32 = 0;
	extern "C" fn store(value: i32) {
		unsafe { WRITTEN = value };
	}

	#[test]
	fn passes_arguments_and_returns() {
		unsafe {
			let r: i32 = call_game(sum as *const (), (1i32, 2u8, -3i16, 40u32));
			assert_eq!(r, 40);
			let r: f64 = call_game(
				mix as *const (),
				(1.5f32, 0.25f64, 3i32, 4usize as *const c_void),
			);
			assert_eq!(r, 22.25);
			let r: f32 = call_game(half as *const (), (5.0f32,));
			assert_eq!(r, 2.5);
			let r: bool = call_game(negate as *const (), (false,));
			assert!(r);
			let r: u64 = call_game(wide as *const (), (3u32,));
			assert_eq!(r, (3 << 32) | 7);
			let r: i32 = call_game(many as *const (), (1, 1, 1, 1, 1, 1, 1, 100.9f64));
			assert_eq!(r, 128);
			call_game::<_, ()>(store as *const (), (42i32,));
			assert_eq!(WRITTEN, 42);
		}
	}
}
