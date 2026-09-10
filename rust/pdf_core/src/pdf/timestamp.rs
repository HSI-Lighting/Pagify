//! A timestamp from an authority that is not us.
//!
//! # Why this one needs the network
//!
//! Every other tool in this program works on the file in front of it. This one
//! cannot: **a time we write ourselves proves nothing**, because we could write
//! any time we liked. What makes a timestamp worth anything is that somebody
//! else — a Time Stamping Authority — signs a statement saying *"this digest
//! existed at this moment"*, and their signature is what a reader checks.
//!
//! So using this sends something out of the machine, and that is worth being
//! exact about:
//!
//! - **What leaves:** a SHA-256 digest of the document. Thirty-two bytes.
//! - **What does not:** the document, its text, its name, its size.
//! - **Where to:** an address the person types. There is no default and no
//!   fallback; nothing is contacted unless somebody names it.
//!
//! A digest is not reversible, but it is not nothing either — an authority that
//! already held a copy of the document could confirm this is that document. If
//! that matters, the answer is not to timestamp.
//!
//! # Plain HTTP, deliberately
//!
//! The token comes back signed by the authority, so transport security adds
//! nothing to its trustworthiness — a token altered in flight fails to verify
//! exactly as one altered anywhere else. RFC 3161 over `http://` is what most
//! authorities offer, and taking it avoids a TLS stack to protect thirty-two
//! bytes of hash that are already public knowledge to the party receiving them.

use crate::error::{PdfError, Result};

/// The DER of an RFC 3161 request for a digest.
///
/// Hand-rolled, because the structure is four fixed fields and the alternative
/// is a derive that would have to be checked against the specification anyway:
///
/// ```text
/// TimeStampReq ::= SEQUENCE {
///   version         INTEGER { v1(1) },
///   messageImprint  SEQUENCE { hashAlgorithm AlgorithmIdentifier,
///                              hashedMessage OCTET STRING },
///   certReq         BOOLEAN }
/// ```
pub fn request(digest: &[u8]) -> Result<Vec<u8>> {
    if digest.len() != 32 {
        return Err(PdfError::InvalidArgument(
            "a timestamp is asked for over a SHA-256 digest".into(),
        ));
    }
    // 2.16.840.1.101.3.4.2.1 — SHA-256, as DER.
    const SHA256_OID: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];

    let algorithm = sequence(&[tlv(0x06, SHA256_OID), tlv(0x05, &[])].concat());
    let imprint = sequence(&[algorithm, tlv(0x04, digest)].concat());

    Ok(sequence(
        &[
            tlv(0x02, &[1]),      // version 1
            imprint,
            tlv(0x01, &[0xFF]),   // certReq TRUE — ask for the certificate back
        ]
        .concat(),
    ))
}

/// One DER tag-length-value.
fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&length(body.len()));
    out.extend_from_slice(body);
    out
}

fn sequence(body: &[u8]) -> Vec<u8> {
    tlv(0x30, body)
}

/// DER length: short form under 128, long form above.
fn length(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let bytes = n.to_be_bytes();
    let significant: Vec<u8> = bytes.iter().copied().skip_while(|b| *b == 0).collect();
    let mut out = vec![0x80 | significant.len() as u8];
    out.extend_from_slice(&significant);
    out
}

/// The token out of an authority's answer.
///
/// ```text
/// TimeStampResp ::= SEQUENCE { status PKIStatusInfo, timeStampToken ContentInfo OPTIONAL }
/// PKIStatusInfo ::= SEQUENCE { status INTEGER, ... }
/// ```
///
/// **The status is read before the token.** An authority that refuses says so
/// in a well-formed answer, and treating that answer's absent token as a
/// malformed reply would blame the wrong thing.
pub fn token_in(response: &[u8]) -> Result<Vec<u8>> {
    let outer = contents_of(response, 0x30)
        .ok_or_else(|| PdfError::InvalidArgument("that is not a timestamp answer".into()))?;

    // The status comes first.
    let (status_body, status_len) = field(outer, 0x30)
        .ok_or_else(|| PdfError::InvalidArgument("the answer carries no status".into()))?;
    let (granted, _) = field(status_body, 0x02)
        .ok_or_else(|| PdfError::InvalidArgument("the status has no value".into()))?;
    let status = granted.first().copied().unwrap_or(255);
    // 0 is granted; 1 is granted with changes. Everything else is a refusal.
    if status > 1 {
        return Err(PdfError::Unsupported(
            "the timestamping authority refused to issue one",
        ));
    }

    let rest = &outer[status_len..];
    let (_, token_len) = field(rest, 0x30)
        .ok_or_else(|| PdfError::InvalidArgument("the answer carries no token".into()))?;
    Ok(rest[..token_len].to_vec())
}

/// The body of the first field, and how many bytes it occupied whole.
fn field(bytes: &[u8], tag: u8) -> Option<(&[u8], usize)> {
    if bytes.first() != Some(&tag) {
        return None;
    }
    let (len, header) = read_length(&bytes[1..])?;
    let start = 1 + header;
    let end = start.checked_add(len)?;
    (end <= bytes.len()).then(|| (&bytes[start..end], end))
}

fn contents_of(bytes: &[u8], tag: u8) -> Option<&[u8]> {
    field(bytes, tag).map(|(body, _)| body)
}

/// A DER length, and how many bytes encoded it.
fn read_length(bytes: &[u8]) -> Option<(usize, usize)> {
    let first = *bytes.first()?;
    if first < 0x80 {
        return Some((first as usize, 1));
    }
    let count = (first & 0x7F) as usize;
    if count == 0 || count > 8 || bytes.len() < 1 + count {
        return None;
    }
    let mut value = 0usize;
    for byte in &bytes[1..1 + count] {
        value = value.checked_mul(256)?.checked_add(*byte as usize)?;
    }
    Some((value, 1 + count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_well_formed_der_over_the_digest() {
        let digest = [9u8; 32];
        let bytes = request(&digest).expect("request");

        // A SEQUENCE, and the digest is inside it.
        assert_eq!(bytes[0], 0x30);
        assert!(
            bytes.windows(32).any(|w| w == digest),
            "the digest is not in the request"
        );
        // Parses as a whole: the outer length accounts for every byte.
        let (body, whole) = field(&bytes, 0x30).expect("sequence");
        assert_eq!(whole, bytes.len(), "the request has trailing rubbish");
        assert!(!body.is_empty());
    }

    /// Only a SHA-256 digest, because that is what the request says it is.
    #[test]
    fn a_digest_of_the_wrong_size_is_refused() {
        assert!(request(&[0u8; 20]).is_err());
        assert!(request(&[]).is_err());
    }

    /// Lengths above 127 use the long form; a request is short, but a token is
    /// not, and the reader has to handle both.
    #[test]
    fn der_lengths_are_read_in_both_forms() {
        assert_eq!(read_length(&[0x05]), Some((5, 1)));
        assert_eq!(read_length(&[0x81, 0x80]), Some((128, 2)));
        assert_eq!(read_length(&[0x82, 0x01, 0x00]), Some((256, 3)));
        // Nonsense comes back as nothing rather than as a huge length.
        assert_eq!(read_length(&[0x89, 1, 2, 3]), None);
        assert_eq!(read_length(&[]), None);
    }

    /// **A refusal is read as a refusal.** An authority that declines answers
    /// properly, and calling that a malformed reply would blame the wrong side.
    #[test]
    fn an_authoritys_refusal_is_reported_as_one() {
        // status = 2 (rejection), no token.
        let status = sequence(&tlv(0x02, &[2]));
        let answer = sequence(&status);
        let problem = token_in(&answer).expect_err("a refusal was read as a token");
        assert!(problem.to_string().contains("refused"), "{problem}");
    }

    #[test]
    fn a_granted_answer_gives_up_its_token() {
        let status = sequence(&tlv(0x02, &[0]));
        // A token is a ContentInfo, which is a SEQUENCE — its innards do not
        // matter here.
        let token = sequence(&tlv(0x06, &[0x2A, 0x86, 0x48]));
        let answer = sequence(&[status, token.clone()].concat());

        assert_eq!(token_in(&answer).expect("token"), token);
    }

    #[test]
    fn rubbish_is_not_read_as_an_answer() {
        assert!(token_in(b"not der at all").is_err());
        assert!(token_in(&[]).is_err());
    }
}

// ------------------------------------------------------------- the network --

use std::io::{Read, Write};

/// Ask an authority for a timestamp over a digest.
///
/// **This is the only function in the program that opens a socket.** It is
/// given an address by name — there is no default, no list, and no fallback —
/// so nothing is contacted unless somebody typed where. See the module note for
/// exactly what leaves the machine.
///
/// `http://` only. The token is signed by the authority, so a transport that
/// could be tampered with cannot make a bad token look good; adding a TLS stack
/// to protect a hash the other party is about to be told anyway would be
/// ceremony.
pub fn ask(authority: &str, digest: &[u8]) -> Result<Vec<u8>> {
    let (host, port, path) = split(authority)?;
    let body = request(digest)?;

    let mut socket = std::net::TcpStream::connect((host.as_str(), port))
        .map_err(|e| PdfError::InvalidArgument(format!("could not reach {host}: {e}")))?;
    // A timestamp is not worth hanging the program for.
    let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(20)));
    let _ = socket.set_write_timeout(Some(std::time::Duration::from_secs(20)));

    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\n\
         Content-Type: application/timestamp-query\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket
        .write_all(head.as_bytes())
        .and_then(|()| socket.write_all(&body))
        .map_err(|e| PdfError::InvalidArgument(format!("could not ask {host}: {e}")))?;

    let mut answer = Vec::new();
    socket
        .read_to_end(&mut answer)
        .map_err(|e| PdfError::InvalidArgument(format!("{host} did not answer: {e}")))?;

    // Past the headers to the body.
    let at = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| PdfError::InvalidArgument(format!("{host} answered with nothing")))?;
    let status = String::from_utf8_lossy(&answer[..answer.iter().position(|b| *b == b'\r').unwrap_or(0)])
        .to_string();
    if !status.contains(" 200") {
        return Err(PdfError::InvalidArgument(format!("{host} said: {status}")));
    }
    token_in(&answer[at + 4..])
}

/// Host, port and path from an address.
///
/// Deliberately strict: an address that is not plainly `http://host/path` is
/// refused rather than guessed at, because guessing here means connecting
/// somewhere nobody asked for.
fn split(authority: &str) -> Result<(String, u16, String)> {
    let rest = authority.strip_prefix("http://").ok_or_else(|| {
        PdfError::InvalidArgument(
            "a timestamping address must begin with http:// — see the note on why \
             https is not needed here"
                .into(),
        )
    })?;
    let (host_part, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    if host_part.is_empty() {
        return Err(PdfError::InvalidArgument("that address has no host".into()));
    }
    let (host, port) = match host_part.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse().map_err(|_| {
                PdfError::InvalidArgument(format!("{p:?} is not a port number"))
            })?,
        ),
        None => (host_part.to_string(), 80u16),
    };
    Ok((host, port, path.to_string()))
}

#[cfg(test)]
mod address_tests {
    use super::*;

    #[test]
    fn an_ordinary_address_is_taken_apart() {
        assert_eq!(
            split("http://timestamp.example.com/tsa").expect("split"),
            ("timestamp.example.com".into(), 80, "/tsa".into())
        );
        // No path is the root.
        assert_eq!(
            split("http://tsa.example.com").expect("split"),
            ("tsa.example.com".into(), 80, "/".into())
        );
        assert_eq!(
            split("http://tsa.example.com:8080/x").expect("split"),
            ("tsa.example.com".into(), 8080, "/x".into())
        );
    }

    /// **Guessing here means connecting somewhere nobody asked for**, so
    /// anything that is not plainly an address is refused.
    #[test]
    fn anything_that_is_not_plainly_an_address_is_refused() {
        for address in [
            "timestamp.example.com",
            "https://timestamp.example.com",
            "ftp://tsa.example.com",
            "",
            "http://",
            "http://host:notaport/",
        ] {
            assert!(split(address).is_err(), "{address:?} was accepted");
        }
    }

    /// And the refusal explains itself rather than saying "invalid".
    #[test]
    fn the_refusal_says_what_is_wanted() {
        let problem = split("https://tsa.example.com").expect_err("accepted").to_string();
        assert!(problem.contains("http://"), "{problem}");
    }
}
