// src-tauri/src/session/poll_reader.rs
//
// poll([target_fd, shutdown_read], -1) 기반 인터럽트 가능 블로킹 리더.
// unix 전용 — 세션 핸드오프(§핵심 1, 2)의 기반 부품. 기존 리더는 블로킹
// `read()`라 인터럽트가 불가능했다: 핸드오프 시 앱 쪽 리더 스레드를 먼저
// 확정적으로 멈추지 않고 fd를 데몬에 넘기면, 같은 마스터 fd를 두 프로세스가
// 동시에 read()해 바이트가 쪼개져 유실될 수 있다(설계 문서 "이중 리더 금지").
// shutdown pipe에 1바이트를 쓰면 poll이 즉시 깨어나고, read()는 Ok(0)(EOF와
// 동일하게 취급)을 반환해 루프를 빠져나간다 — target_fd 자체는 절대 만지지
// 않으므로 커널 tty 버퍼에 남은 미독 바이트는 fd를 이어받는 쪽(데몬 또는
// 입양된 세션)이 그대로 이어 읽는다.
//
// `PollReader`는 대상 fd를 빌려 쓸 뿐 소유하지 않는다 — 닫기는 항상 그 fd를
// 실제로 소유한 컨트롤 구조체(RealControl/AdoptedControl/데몬 SessionEntry)의
// 책임이다. 이 분리 덕분에 리더 스레드가 먼저 죽어도(인터럽트) fd 자체는
// 멀쩡히 남아 다음 리더가 이어받을 수 있다.

use std::io::{self, Read};
use std::os::unix::io::RawFd;

use nix::poll::{poll, PollFd, PollFlags};
use nix::unistd::{close, pipe, read as nix_read, write as nix_write};

pub struct PollReader {
    fd: RawFd,
    shutdown_read: RawFd,
}

pub struct ReaderInterrupt {
    shutdown_write: RawFd,
}

impl ReaderInterrupt {
    /// 리더 스레드를 깨워 EOF처럼 멈추게 한다. 두 번째 이후 호출도 안전
    /// (파이프에 바이트가 이미 남아 있어도 재기록만 될 뿐 무해).
    pub fn interrupt(&self) {
        let _ = nix_write(self.shutdown_write, &[0u8]);
    }
}

impl Drop for ReaderInterrupt {
    fn drop(&mut self) {
        let _ = close(self.shutdown_write);
    }
}

impl Drop for PollReader {
    fn drop(&mut self) {
        let _ = close(self.shutdown_read);
    }
}

/// `fd`는 빌려 쓴다(닫지 않음) — 소유권은 언제나 호출자 쪽 컨트롤 구조체에 있다.
pub fn spawn(fd: RawFd) -> io::Result<(PollReader, ReaderInterrupt)> {
    let (shutdown_read, shutdown_write) = pipe().map_err(io::Error::from)?;
    Ok((
        PollReader { fd, shutdown_read },
        ReaderInterrupt { shutdown_write },
    ))
}

impl Read for PollReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut fds = [
                PollFd::new(self.fd, PollFlags::POLLIN),
                PollFd::new(self.shutdown_read, PollFlags::POLLIN),
            ];
            match poll(&mut fds, -1) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(io::Error::from(e)),
            }

            // shutdown을 먼저 확인 — 인터럽트 요청이면 target_fd에 남은 커널
            // 버퍼 바이트는 건드리지 않고 그대로 EOF처럼 종료한다.
            if fds[1]
                .revents()
                .map(|r| !r.is_empty())
                .unwrap_or(false)
            {
                return Ok(0);
            }

            let revents = fds[0].revents().unwrap_or(PollFlags::empty());
            if revents.contains(PollFlags::POLLIN) {
                return match nix_read(self.fd, buf) {
                    Ok(n) => Ok(n),
                    Err(nix::errno::Errno::EAGAIN) | Err(nix::errno::Errno::EINTR) => continue,
                    // EIO: macOS가 슬레이브 쪽이 전부 닫혔을 때 마스터 read에서
                    // 흔히 돌려주는 코드 — 프로세스 종료를 EOF와 동일하게 취급.
                    Err(nix::errno::Errno::EIO) => return Ok(0),
                    Err(e) => Err(io::Error::from(e)),
                };
            }
            if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL) {
                return Ok(0);
            }
            // 어느 쪽도 아니면(허위 기상) 다시 poll.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    use std::time::Duration;

    #[test]
    fn interrupt_stops_reader_without_consuming_pending_bytes() {
        // pipe()의 쓰기 끝에 데이터를 남겨둔 채 인터럽트하면, 리더는 그 데이터를
        // 건드리지 않고 즉시 Ok(0)을 반환해야 한다 — "이중 리더 금지" 계약의
        // 핵심(잔여 바이트는 다음 리더가 그대로 읽는다).
        let (target_read, target_write) = pipe().unwrap();
        let (mut reader, interrupt) = spawn(target_read).unwrap();

        // 아직 아무 데이터도 쓰지 않은 상태에서 인터럽트 -> 리더는 블록되지
        // 않고 즉시 깨어나 Ok(0)을 반환해야 한다.
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 16];
            reader.read(&mut buf)
        });
        std::thread::sleep(Duration::from_millis(20));
        interrupt.interrupt();
        let result = handle.join().unwrap().unwrap();
        assert_eq!(result, 0, "interrupt must surface as Ok(0), not block forever");

        // target_write에 남겨둔 fd를 통해 별도 리더가 여전히 읽을 수 있음을
        // 확인 -- target_read 자체는 인터럽트로 인해 닫히지 않았다.
        nix_write(target_write, b"later").unwrap();
        let mut buf = [0u8; 16];
        let n = nix_read(target_read, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"later");

        let _ = close(target_write);
        let _ = close(target_read);
    }

    #[test]
    fn reads_data_when_available_before_any_interrupt() {
        let (target_read, target_write) = pipe().unwrap();
        let (mut reader, _interrupt) = spawn(target_read).unwrap();

        let mut writer = unsafe { std::fs::File::from_raw_fd(target_write) };
        writer.write_all(b"hello").unwrap();
        drop(writer);

        let mut buf = [0u8; 16];
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");

        let _ = close(target_read);
    }
}
