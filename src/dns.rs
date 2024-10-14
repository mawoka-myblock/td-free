use std::net::UdpSocket;

pub fn start_dns_hijack_server(portal_ip: [u8; 4]) -> std::io::Result<()> {
    // Bind to UDP port 53 to listen for DNS requests from any IP
    let socket = UdpSocket::bind("0.0.0.0:53")?;
    println!("DNS server listening on port 53");

    loop {
        // Create a buffer to receive data
        let mut buffer = [0u8; 512];

        // Receive data from a client
        let (size, src) = socket.recv_from(&mut buffer)?;
        println!("Received DNS request from {}", src);

        // Create a DNS response packet
        let response = create_dns_response(&buffer[0..size], portal_ip);

        // Send the DNS response back to the client, regardless of the target DNS server
        socket.send_to(&response, src)?;
    }
}

/// Create a simple DNS response that always points to `portal_ip`
fn create_dns_response(request: &[u8], portal_ip: [u8; 4]) -> Vec<u8> {
    let mut response = Vec::new();

    // Copy the DNS header (12 bytes)
    response.extend_from_slice(&request[0..12]);

    // Set response flags: QR (1), Opcode (0), AA (1), TC (0), RD (1), RA (1)
    response[2] = 0x81; // 10000001 - QR (1) + AA (1)
    response[3] = 0x80; // 10000000 - RD (1) + RA (1)

    // Copy the QDCOUNT (Question Count)
    response.extend_from_slice(&request[4..6]);

    // Set ANCOUNT (Answer Count) to 1
    response.extend_from_slice(&[0x00, 0x01]);

    // NSCOUNT (Authority RRs) and ARCOUNT (Additional RRs) set to 0
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    // Copy the original Question section
    let question_section_len = request.len() - 12;
    response.extend_from_slice(&request[12..]);

    // Add the answer section
    // Name pointer (0xc00c) points to the Question section
    response.extend_from_slice(&[0xc0, 0x0c]);

    // Type (A record = 0x0001)
    response.extend_from_slice(&[0x00, 0x01]);

    // Class (IN = 0x0001)
    response.extend_from_slice(&[0x00, 0x01]);

    // TTL (time to live, 60 seconds)
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x3c]);

    // Data length (IPv4 = 4 bytes)
    response.extend_from_slice(&[0x00, 0x04]);

    // The IP address to redirect (portal_ip)
    response.extend_from_slice(&portal_ip);

    response
}
