// Copyright 2017 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

use std::io::{Result, Write};
use std::sync::Arc;
use std::option::Option;

use rocksdb::RateLimiter;

const PRIORITY_HIGH: u8 = 1;
const REFILL_PERIOD: i64 = 100 * 1000;
const FARENESS: i32 = 10;
const SNAP_MAX_BYTES_PER_TIME: i64 = 4 * 1024 * 1024;
pub const DEFAULT_SNAP_MAX_BYTES_PER_SEC: u64 = 30 * 1024 * 1024;

pub struct IOLimiter {
    inner: RateLimiter,
}

impl IOLimiter {
    pub fn new(bytes_per_sec: u64) -> IOLimiter {
        IOLimiter {
            inner: RateLimiter::new(bytes_per_sec as i64, REFILL_PERIOD, FARENESS),
        }
    }

    pub fn set_bytes_per_second(&self, bytes_per_sec: i64) {
        self.inner.set_bytes_per_second(bytes_per_sec)
    }

    pub fn request(&self, bytes: i64) {
        self.inner.request(bytes, PRIORITY_HIGH)
    }

    pub fn get_max_bytes_per_time(&self) -> i64 {
        if self.inner.get_singleburst_bytes() > SNAP_MAX_BYTES_PER_TIME {
            SNAP_MAX_BYTES_PER_TIME
        } else {
            self.inner.get_singleburst_bytes()
        }
    }

    pub fn get_total_bytes_through(&self) -> i64 {
        self.inner.get_total_bytes_through(PRIORITY_HIGH)
    }

    pub fn get_bytes_per_second(&self) -> i64 {
        self.inner.get_bytes_per_second()
    }

    pub fn get_total_requests(&self) -> i64 {
        self.inner.get_total_requests(PRIORITY_HIGH)
    }
}

pub struct LimitWriter<'a, T: Write + 'a> {
    limiter: Option<Arc<IOLimiter>>,
    writer: &'a mut T,
}

impl<'a, T: Write + 'a> LimitWriter<'a, T> {
    pub fn new(limiter: Option<Arc<IOLimiter>>, writer: &'a mut T) -> LimitWriter<'a, T> {
        LimitWriter {
            limiter: limiter,
            writer: writer,
        }
    }
}

impl<'a, T: Write + 'a> Write for LimitWriter<'a, T> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let total = buf.len();
        if let Some(ref limiter) = self.limiter {
            let single = limiter.get_max_bytes_per_time() as usize;
            let mut curr = 0;
            let mut end;
            while curr < total {
                if curr + single >= total {
                    end = total;
                } else {
                    end = curr + single;
                }
                limiter.request((end - curr) as i64);
                self.writer.write_all(&buf[curr..end])?;
                curr = end;
            }
        } else {
            self.writer.write_all(buf)?;
        }
        Ok(total)
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::sync::Arc;

    use super::{IOLimiter, LimitWriter, SNAP_MAX_BYTES_PER_TIME};

    #[test]
    fn test_io_limiter() {
        let limiter = IOLimiter::new(10 * 1024 * 1024);
        assert!(limiter.get_max_bytes_per_time() <= SNAP_MAX_BYTES_PER_TIME);

        limiter.set_bytes_per_second(20 * 1024 * 1024);
        assert_eq!(limiter.get_bytes_per_second(), 20 * 1024 * 1024);

        assert_eq!(limiter.get_total_bytes_through(), 0);

        limiter.request(1024 * 1024);
        assert_eq!(limiter.get_total_bytes_through(), 1024 * 1024);

        assert_eq!(limiter.get_total_requests(), 1);
    }

    #[test]
    fn test_limit_writer() {
        let mut file = File::create("/tmp/test_limit_writer.txt").unwrap();
        let mut limit_writer = LimitWriter::new(Some(Arc::new(IOLimiter::new(1024))), &mut file);

        let mut s = String::new();
        for _ in 0..100 {
            s.push_str("Hello, World!");
        }
        limit_writer.write_all(s.as_bytes()).unwrap();
        limit_writer.flush().unwrap();

        let mut file = File::open("/tmp/test_limit_writer.txt").unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, s);
    }
}
