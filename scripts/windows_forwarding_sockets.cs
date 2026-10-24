// Independent owned OS socket observer for the M118 technical host. C# 5.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;

public sealed class MioForwardSockets : IDisposable
{
    private readonly object gate = new object();
    private readonly TcpListener tcp;
    private readonly UdpClient udp;
    private readonly Thread tcpReader;
    private readonly Thread udpReader;
    private readonly List<Thread> workers = new List<Thread>();
    private readonly HashSet<TcpClient> targetClients = new HashSet<TcpClient>();
    private readonly Dictionary<int, Socket> clients = new Dictionary<int, Socket>();
    private readonly Dictionary<int, string> protocols = new Dictionary<int, string>();
    private readonly List<string> errors = new List<string>();
    private volatile bool stopping;
    private int next;
    private long tcpBytes, udpBytes, udpDatagrams;
    private int tcpLength, tcpSeed, udpLength, udpSeed;
    private long tcpCaseBytes, udpCaseDatagrams;
    private int udpReplyLength = -1, udpReplySeed;
    private bool udpEchoAfterReply;
    private long udpCaseReplies;
    private bool tcpReplyAfterEof;
    private int tcpReplyLength, tcpReplySeed;
    private long tcpEofReplies;
    private bool tcpExact = true, udpExact = true;
    public string TcpAddress { get; private set; }
    public string UdpAddress { get; private set; }

    public MioForwardSockets()
    {
        tcp = new TcpListener(IPAddress.IPv6Loopback, 0);
        try
        {
            tcp.Start(16);
            udp = new UdpClient(new IPEndPoint(IPAddress.IPv6Loopback, 0));
            udp.Client.ReceiveTimeout = 100;
            TcpAddress = tcp.LocalEndpoint.ToString();
            UdpAddress = udp.Client.LocalEndPoint.ToString();
            tcpReader = new Thread(ReadTcp); udpReader = new Thread(ReadUdp);
            tcpReader.IsBackground = udpReader.IsBackground = true;
            tcpReader.Start(); udpReader.Start();
        }
        catch
        {
            stopping = true; tcp.Stop();
            if (udp != null) udp.Close();
            if (tcpReader != null && tcpReader.IsAlive) tcpReader.Join(2000);
            if (udpReader != null && udpReader.IsAlive) udpReader.Join(2000);
            throw;
        }
    }
    private void Error(Exception failure)
    {
        lock (gate) { if (!stopping && errors.Count < 16) errors.Add(failure.GetType().Name); }
    }
    private void CleanupError(Exception failure)
    {
        lock (gate) { if (errors.Count < 16) errors.Add("cleanup:" + failure.GetType().Name); }
    }
    public void Expect(string protocol, int length, int seed)
    {
        if (length < 0 || length > (protocol == "udp" ? 65527 : 1048576) || seed < 0 || seed > 250)
            throw new ArgumentException("invalid owned test case");
        lock (gate)
        {
            if (protocol == "tcp") { tcpLength = length; tcpSeed = seed; tcpCaseBytes = 0; tcpExact = true; tcpReplyAfterEof = false; tcpEofReplies = 0; }
            else if (protocol == "udp") { udpLength = length; udpSeed = seed; udpCaseDatagrams = 0; udpExact = true; udpReplyLength = -1; udpCaseReplies = 0; }
            else throw new ArgumentException("unknown protocol");
        }
    }
    public void ExpectEofReply(int length, int seed, int responseLength, int responseSeed)
    {
        if (responseLength < 0 || responseLength > 1048576 || responseSeed < 0 || responseSeed > 250)
            throw new ArgumentException("invalid owned EOF response");
        Expect("tcp", length, seed);
        lock (gate) { tcpReplyAfterEof = true; tcpReplyLength = responseLength; tcpReplySeed = responseSeed; }
    }
    public void ExpectUdpReply(int length, int seed, int responseLength, int responseSeed, bool echoAfterReply)
    {
        if (responseLength < 0 || responseLength > 65527 || responseSeed < 0 || responseSeed > 250)
            throw new ArgumentException("invalid owned UDP response");
        Expect("udp", length, seed);
        lock (gate) { udpReplyLength = responseLength; udpReplySeed = responseSeed; udpEchoAfterReply = echoAfterReply; }
    }
    private void ReadTcp()
    {
        while (!stopping)
        {
            TcpClient client;
            try { client = tcp.AcceptTcpClient(); }
            catch (Exception failure) { Error(failure); break; }
            lock (gate)
            {
                if (stopping || targetClients.Count >= 16) { client.Close(); continue; }
                targetClients.Add(client);
                Thread worker = new Thread(delegate() { EchoTcp(client); });
                worker.IsBackground = true; workers.Add(worker); worker.Start();
            }
        }
    }
    private void EchoTcp(TcpClient client)
    {
        try
        {
            client.ReceiveTimeout = 100; client.SendTimeout = 10000;
            using (NetworkStream stream = client.GetStream())
            {
                byte[] buffer = new byte[65536];
                while (!stopping)
                {
                    int count;
                    try { count = stream.Read(buffer, 0, buffer.Length); }
                    catch (System.IO.IOException failure)
                    {
                        SocketException socket = failure.InnerException as SocketException;
                        if (socket != null && socket.SocketErrorCode == SocketError.TimedOut) continue;
                        if (socket != null && (socket.SocketErrorCode == SocketError.ConnectionReset || socket.SocketErrorCode == SocketError.ConnectionAborted)) break;
                        throw;
                    }
                    if (count == 0)
                    {
                        bool replyAfterEof; int length, seed;
                        lock (gate) { replyAfterEof = tcpReplyAfterEof; length = tcpReplyLength; seed = tcpReplySeed; }
                        if (replyAfterEof)
                        {
                            byte[] reply = new byte[length];
                            for (int index = 0; index < reply.Length; index++) reply[index] = (byte)((index + seed) % 251);
                            stream.Write(reply, 0, reply.Length);
                            client.Client.Shutdown(SocketShutdown.Send);
                            lock (gate) { tcpEofReplies++; }
                        }
                        break;
                    }
                    bool echo;
                    lock (gate)
                    {
                        for (int index = 0; index < count; index++)
                            if (tcpCaseBytes + index >= tcpLength || buffer[index] != (byte)((tcpCaseBytes + index + tcpSeed) % 251)) tcpExact = false;
                        tcpCaseBytes += count; tcpBytes += count;
                        echo = !tcpReplyAfterEof;
                    }
                    if (echo) stream.Write(buffer, 0, count);
                }
            }
        }
        catch (Exception failure) { Error(failure); }
        finally { client.Close(); lock (gate) { targetClients.Remove(client); workers.Remove(Thread.CurrentThread); } }
    }
    private void ReadUdp()
    {
        while (!stopping)
        {
            try
            {
                IPEndPoint peer = new IPEndPoint(IPAddress.IPv6Loopback, 0);
                byte[] data = udp.Receive(ref peer);
                int replyLength, replySeed; bool echoAfterReply;
                lock (gate)
                {
                    if (data.Length != udpLength) udpExact = false;
                    for (int index = 0; index < data.Length; index++) if (data[index] != (byte)((index + udpSeed) % 251)) udpExact = false;
                    udpCaseDatagrams++; udpDatagrams++; udpBytes += data.Length;
                    replyLength = udpReplyLength; replySeed = udpReplySeed; echoAfterReply = udpEchoAfterReply;
                }
                if (replyLength >= 0)
                {
                    byte[] reply = new byte[replyLength];
                    for (int index = 0; index < reply.Length; index++) reply[index] = (byte)((index + replySeed) % 251);
                    if (udp.Send(reply, reply.Length, peer) != reply.Length) throw new System.IO.IOException("incomplete owned UDP send");
                    lock (gate) { udpCaseReplies++; }
                }
                if (replyLength < 0 || echoAfterReply) udp.Send(data, data.Length, peer);
            }
            catch (SocketException failure) { if (failure.SocketErrorCode != SocketError.TimedOut) { Error(failure); break; } }
            catch (Exception failure) { Error(failure); break; }
        }
    }
    private static IPEndPoint Loopback(string address)
    {
        int colon = address.LastIndexOf(':');
        if (colon <= 0) throw new ArgumentException("invalid socket address");
        IPAddress host = IPAddress.Parse(address.Substring(0, colon).Trim('[', ']'));
        int port = int.Parse(address.Substring(colon + 1));
        if (!IPAddress.IsLoopback(host) || port < 1 || port > 65535) throw new ArgumentException("only owned loopback sockets");
        return new IPEndPoint(host, port);
    }
    public int OpenClient(string protocol, string address)
    {
        if (protocol != "tcp" && protocol != "udp") throw new ArgumentException("unknown protocol");
        IPEndPoint endpoint = Loopback(address);
        Socket socket = new Socket(endpoint.AddressFamily, protocol == "tcp" ? SocketType.Stream : SocketType.Dgram,
                                   protocol == "tcp" ? ProtocolType.Tcp : ProtocolType.Udp);
        int id;
        lock (gate)
        {
            if (stopping || clients.Count >= 16) { socket.Close(); throw new InvalidOperationException("owned client bound"); }
            id = checked(++next); clients.Add(id, socket); protocols.Add(id, protocol);
        }
        try
        {
            socket.SendTimeout = socket.ReceiveTimeout = 10000;
            IAsyncResult connecting = socket.BeginConnect(endpoint, null, null);
            using (connecting.AsyncWaitHandle)
            {
                if (!connecting.AsyncWaitHandle.WaitOne(5000)) { socket.Close(); throw new TimeoutException("owned local connection deadline"); }
                socket.EndConnect(connecting);
            }
            return id;
        }
        catch { CloseClient(id); throw; }
    }
    public Dictionary<string, object> Exchange(int id, int length, int seed)
    {
        return ExchangeCore(id, length, seed, length, seed, false);
    }
    public Dictionary<string, object> ExchangeHalfClosed(int id, int length, int seed, int responseLength, int responseSeed)
    {
        return ExchangeCore(id, length, seed, responseLength, responseSeed, true);
    }
    public Dictionary<string, object> SendDatagram(int id, int length, int seed)
    {
        Socket socket;
        lock (gate) { if (protocols[id] != "udp") throw new ArgumentException("UDP client required"); socket = clients[id]; }
        if (length < 0 || length > 65527 || seed < 0 || seed > 250) throw new ArgumentException("owned UDP payload bound");
        byte[] data = new byte[length];
        for (int index = 0; index < data.Length; index++) data[index] = (byte)((index + seed) % 251);
        int sent = socket.Send(data, 0, data.Length, SocketFlags.None);
        if (sent != data.Length) throw new System.IO.IOException("incomplete owned UDP send");
        return new Dictionary<string, object> { { "sent", sent }, { "delivery_confirmed", false } };
    }
    private Dictionary<string, object> ExchangeCore(int id, int length, int seed, int responseLength, int responseSeed, bool halfClose)
    {
        Socket socket; string protocol;
        lock (gate) { socket = clients[id]; protocol = protocols[id]; }
        if (length < 0 || length > (protocol == "udp" ? 65527 : 1048576) || seed < 0 || seed > 250) throw new ArgumentException("owned payload bound");
        if (responseLength < 0 || responseLength > (protocol == "udp" ? 65527 : 1048576) || responseSeed < 0 || responseSeed > 250 || (halfClose && protocol != "tcp"))
            throw new ArgumentException("owned response bound");
        byte[] data = new byte[length]; for (int index = 0; index < length; index++) data[index] = (byte)((index + seed) % 251);
        Task upload = Task.Run(delegate()
        {
            int sent = 0;
            do
            {
                int count = socket.Send(data, sent, data.Length - sent, SocketFlags.None);
                if (count == 0 && sent < length) throw new System.IO.EndOfStreamException();
                sent += count;
            } while (protocol == "tcp" && sent < length);
            if (halfClose) socket.Shutdown(SocketShutdown.Send);
        });
        try
        {
            byte[] response = new byte[protocol == "udp" ? 65536 : responseLength]; int received = 0;
            while (protocol == "udp" || received < responseLength)
            {
                int count = socket.Receive(response, received, response.Length - received, SocketFlags.None);
                if (protocol == "tcp" && count == 0 && received < responseLength) throw new System.IO.EndOfStreamException();
                received += count;
                if (protocol == "udp") break;
            }
            if (!upload.Wait(10000)) throw new TimeoutException("owned upload deadline");
            bool eof = false;
            if (halfClose) { eof = socket.Receive(new byte[1], 0, 1, SocketFlags.None) == 0; if (!eof) throw new System.IO.InvalidDataException("extra EOF response"); }
            bool exact = received == responseLength;
            for (int index = 0; index < received && index < responseLength; index++) exact &= response[index] == (byte)((index + responseSeed) % 251);
            return new Dictionary<string, object> { { "length", received }, { "exact", exact }, { "write_half_closed", halfClose }, { "eof", eof } };
        }
        catch
        {
            CloseClient(id);
            try { if (!upload.Wait(2000)) CleanupError(new TimeoutException("owned upload not joined")); }
            catch (AggregateException) { /* The original failed operation is retained by the controller. */ }
            throw;
        }
    }
    public void CloseClient(int id)
    {
        lock (gate) { Socket socket; if (clients.TryGetValue(id, out socket)) { clients.Remove(id); protocols.Remove(id); socket.Close(); } }
    }
    public Dictionary<string, object> Snapshot()
    {
        lock (gate) return new Dictionary<string, object> {
            { "tcp_bytes", tcpBytes }, { "udp_bytes", udpBytes }, { "udp_datagrams", udpDatagrams },
            { "tcp_case_bytes", tcpCaseBytes }, { "tcp_case_exact", tcpExact && tcpCaseBytes == tcpLength },
            { "tcp_case_eof_replies", tcpEofReplies }, { "tcp_reply_after_eof", tcpReplyAfterEof },
            { "udp_case_datagrams", udpCaseDatagrams }, { "udp_case_exact", udpExact },
            { "udp_case_replies", udpCaseReplies }, { "udp_reply_length", udpReplyLength },
            { "active_target_clients", targetClients.Count }, { "active_application_clients", clients.Count },
            { "errors", errors.ToArray() }, { "terminal", stopping && !tcpReader.IsAlive && !udpReader.IsAlive && workers.Count == 0 && clients.Count == 0 }
        };
    }
    public void Dispose()
    {
        Thread[] pending; TcpClient[] active; Socket[] applications;
        lock (gate)
        {
            stopping = true; pending = workers.ToArray(); active = new TcpClient[targetClients.Count]; targetClients.CopyTo(active);
            applications = new Socket[clients.Count]; clients.Values.CopyTo(applications, 0); clients.Clear(); protocols.Clear();
        }
        try { tcp.Stop(); } catch (Exception failure) { CleanupError(failure); }
        try { udp.Close(); } catch (Exception failure) { CleanupError(failure); }
        foreach (Socket socket in applications) { try { socket.Close(); } catch (Exception failure) { CleanupError(failure); } }
        foreach (TcpClient client in active) { try { client.Close(); } catch (Exception failure) { CleanupError(failure); } }
        Stopwatch watch = Stopwatch.StartNew();
        foreach (Thread thread in new Thread[] { tcpReader, udpReader }) thread.Join(Math.Max(0, 5000 - (int)watch.ElapsedMilliseconds));
        foreach (Thread thread in pending) thread.Join(Math.Max(0, 5000 - (int)watch.ElapsedMilliseconds));
    }
}
