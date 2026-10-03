// Real owned broker proxy for the mixed-carrier regression only.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer as httpsServer } from 'node:https';
import { connect } from 'node:net';
import { join } from 'node:path';
import { createServer as tlsServer } from 'node:tls';
export async function startMixedMqttProxy({fixture,broker,report,listen,ownedSockets,dropPublish}) {
 const [brokerHost,brokerPort]=broker.split(':');report.mqtt={proxy:[],tls:[]};
function track(socket) {
  ownedSockets.add(socket);
  socket.on('error', () => {});
  socket.once('close', () => ownedSockets.delete(socket));
  return socket;
}
function mqttReader(consume) {
  let pending = Buffer.alloc(0);
  return bytes => {
    pending = Buffer.concat([pending, bytes]);
    if (pending.length > 16384) throw new Error('fixture MQTT buffer limit');
    while (pending.length >= 2) {
      let length = 0;
      let offset = 1;
      let digit;
      do {
        if (offset >= pending.length) return;
        if (offset > 4) throw new Error('fixture malformed MQTT length');
        digit = pending[offset++];
        length += (digit & 127) * (128 ** (offset - 2));
      } while (digit & 128);
      if (offset + length > 4096) throw new Error('fixture MQTT packet limit');
      if (pending.length < offset + length) return;
      const packet = Buffer.from(pending.subarray(0, offset + length));
      pending = pending.subarray(offset + length);
      consume(packet, offset);
    }
  };
}
function bridge(client, send, mode, label) {
  const broker = track(connect({ host: brokerHost, port: Number(brokerPort) }));
  const entry = { label, mode, packetsFromClient: [], packetsFromBroker: [], publishCount: 0 };
  report.mqtt.proxy.push(entry);
  const outgoing = mqttReader((packet, header) => {
    const kind = packet[0] >> 4;
    entry.packetsFromClient.push({ kind, bytes: packet.length, atMs: Date.now() });
    if (kind === 3) entry.publishCount++;
    if (kind === 3 && dropPublish()) return;
    broker.write(packet);
  });
  const incoming = mqttReader((packet, header) => {
    const kind = packet[0] >> 4;
    entry.packetsFromBroker.push({ kind, bytes: packet.length, atMs: Date.now() });
    send(packet);
  });
  broker.on('data', bytes => { try { incoming(bytes); } catch { client.destroy(); broker.destroy(); } });
  broker.on('error', () => client.destroy());
  broker.on('close', () => client.destroy());
  client.on('close', () => { entry.closedMs = Date.now(); broker.destroy(); });
  return bytes => { try { outgoing(bytes); } catch { client.destroy(); broker.destroy(); } };
}
function websocketFrame(bytes, opcode = 2) {
  const header = bytes.length < 126 ? Buffer.from([0x80 | opcode, bytes.length]) :
    Buffer.from([0x80 | opcode, 126, bytes.length >> 8, bytes.length & 255]);
  return Buffer.concat([header, bytes]);
}
function websocketReader(socket, receive) {
  let pending = Buffer.alloc(0);
  return bytes => {
    pending = Buffer.concat([pending, bytes]);
    if (pending.length > 16384) return socket.destroy();
    while (pending.length >= 2) {
      const opcode = pending[0] & 15;
      if (!(pending[0] & 128) || !(pending[1] & 128)) return socket.destroy();
      let length = pending[1] & 127;
      let offset = 2;
      if (length === 127) return socket.destroy();
      if (length === 126) {
        if (pending.length < 4) return;
        length = pending.readUInt16BE(2);
        offset = 4;
      }
      if (length > 4096) return socket.destroy();
      if (pending.length < offset + 4 + length) return;
      const mask = pending.subarray(offset, offset + 4);
      const frame = Buffer.from(pending.subarray(offset + 4, offset + 4 + length));
      for (let i = 0; i < frame.length; i++) frame[i] ^= mask[i % 4];
      pending = pending.subarray(offset + 4 + length);
      if (opcode === 8) { socket.end(websocketFrame(Buffer.alloc(0), 8)); return; }
      if (opcode === 9) socket.write(websocketFrame(frame, 10));
      else if (opcode === 2) receive(frame);
      else return socket.destroy();
    }
  };
}
    const tlsOptions = { key: readFileSync(join(fixture, 'leaf.key')), cert: readFileSync(join(fixture, 'leaf.pem')) };
    const native = tlsServer(tlsOptions, socket => {
      track(socket);
      report.mqtt.tls.push({ role: 'native', protocol: socket.getProtocol(), cipher: socket.getCipher().standardName, atMs: Date.now() });
      const receive = bridge(socket, bytes => socket.write(bytes), 'normal', 'native-normal');
      socket.on('data', receive);
    });
    const nativePort = await listen(native);
    const wss = httpsServer(tlsOptions, (request, response) => response.writeHead(404).end());
    wss.on('upgrade', (request, socket, head) => {
      track(socket);
      const mode = request.url.split('/').at(-1);
      if (mode !== 'normal'
          || request.headers['sec-websocket-protocol'] !== 'mqtt') return socket.destroy();
      const accept = createHash('sha1').update(request.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: mqtt\r\n\r\n`);
      report.mqtt.tls.push({ role: 'browser', protocol: socket.getProtocol(), cipher: socket.getCipher().standardName, atMs: Date.now() });
      const receive = bridge(socket, bytes => socket.write(websocketFrame(bytes)), mode, `browser-${mode}`);
      const read = websocketReader(socket, receive);
      socket.on('data', read);
      if (head.length) read(head);
    });
    const wssPort = await listen(wss);

 return {logical_id:'owned-mqtt-mixed-retry',native_url:`mqtts://localhost:${nativePort}/`,wss_url:`wss://localhost:${wssPort}/mqtt/normal`};
}
