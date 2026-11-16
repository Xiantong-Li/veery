use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

struct Internet(*mut core::ffi::c_void);

impl Drop for Internet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// WinHTTP GET，返回状态码与二进制响应体；limit 是体积上限，超过即失败。
pub fn get(host: &str, path: &str, limit: usize) -> Result<(u16, Vec<u8>), ()> {
    unsafe {
        let session = WinHttpOpen(
            windows::core::w!("Veery"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            0,
        );
        if session.is_null() {
            return Err(());
        }
        let session = Internet(session);
        let _ = WinHttpSetTimeouts(session.0, 4000, 4000, 8000, 8000);
        let host_wide: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();
        let connect = WinHttpConnect(
            session.0,
            windows::core::PCWSTR(host_wide.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        );
        if connect.is_null() {
            return Err(());
        }
        let connect = Internet(connect);
        let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let request = WinHttpOpenRequest(
            connect.0,
            windows::core::w!("GET"),
            windows::core::PCWSTR(path_wide.as_ptr()),
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err(());
        }
        let request = Internet(request);
        let headers: Vec<u16> = "User-Agent: Veery\r\n".encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(|_| ())?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| ())?;
        let mut status = 0u32;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            windows::core::PCWSTR(std::ptr::null()),
            Some(&mut status as *mut u32 as *mut core::ffi::c_void),
            &mut status_len,
            std::ptr::null_mut(),
        )
        .map_err(|_| ())?;
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(|_| ())?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > limit {
                return Err(());
            }
        }
        Ok((status as u16, body))
    }
}