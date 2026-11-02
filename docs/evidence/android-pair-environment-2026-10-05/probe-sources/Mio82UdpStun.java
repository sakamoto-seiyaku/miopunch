import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.security.SecureRandom;
import java.util.Arrays;

/** Exactly one IPv4 STUN datagram on the requested physical source address. */
public final class Mio82UdpStun {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("physical IPv4 required");
        long started = System.nanoTime();
        int exit = 1;
        DatagramSocket socket = null;
        try {
            InetAddress server = InetAddress.getByName("74.125.250.129");
            socket = new DatagramSocket(new InetSocketAddress(args[0], 0));
            socket.setSoTimeout(3000);
            byte[] transaction = new byte[12];
            new SecureRandom().nextBytes(transaction);
            byte[] request = ByteBuffer.allocate(20).putShort((short) 1)
                .putShort((short) 0).putInt(0x2112a442).put(transaction).array();
            socket.send(new DatagramPacket(request, request.length, server, 19302));
            System.out.println("source=" + socket.getLocalAddress().getHostAddress()
                + ":" + socket.getLocalPort());
            DatagramPacket reply = new DatagramPacket(new byte[2048], 2048);
            socket.receive(reply);
            byte[] bytes = reply.getData();
            ByteBuffer buffer = ByteBuffer.wrap(bytes, 0, reply.getLength());
            boolean valid = reply.getAddress().equals(server) && reply.getPort() == 19302
                && reply.getLength() >= 20 && buffer.getShort(0) == 0x101
                && (buffer.getShort(2) & 0xffff) == reply.getLength() - 20
                && buffer.getInt(4) == 0x2112a442
                && Arrays.equals(transaction, Arrays.copyOfRange(bytes, 8, 20));
            System.out.println("transactionAndSourceMatched=" + valid);
            System.out.println("responsePeer=" + reply.getAddress().getHostAddress()
                + ":" + reply.getPort());
            boolean mapped = false;
            for (int offset = 20; valid && offset + 4 <= reply.getLength();) {
                int kind = buffer.getShort(offset) & 0xffff;
                int length = buffer.getShort(offset + 2) & 0xffff;
                if (offset + 4 + length > reply.getLength()) throw new IllegalStateException("truncated attribute");
                if (kind == 0x20 && length == 8 && bytes[offset + 5] == 1) {
                    int port = (buffer.getShort(offset + 6) & 0xffff) ^ 0x2112;
                    byte[] address = ByteBuffer.allocate(4)
                        .putInt(buffer.getInt(offset + 8) ^ 0x2112a442).array();
                    System.out.println("mapped=" + InetAddress.getByAddress(address).getHostAddress() + ":" + port);
                    mapped = true;
                }
                offset += 4 + ((length + 3) / 4) * 4;
            }
            if (valid && mapped) exit = 0;
        } catch (Exception error) {
            System.out.println("failureClass=" + error.getClass().getSimpleName());
        } finally {
            System.out.println("callerElapsedMs=" + (System.nanoTime() - started) / 1000000.0);
            long cleanup = System.nanoTime();
            if (socket != null) socket.close();
            System.out.println("cleanupElapsedMs=" + (System.nanoTime() - cleanup) / 1000000.0);
            System.out.println("ownedSocketClosed=" + (socket != null && socket.isClosed()));
        }
        System.exit(exit);
    }
}
