import { createInterface } from 'node:readline';
import { writeFileSync } from 'node:fs';
import { startMixedMqttProxy } from './owned-mixed-mqtt-proxy.mjs';

const report = { startedUtc: new Date().toISOString() };
const sockets = new Set();
const servers = [];
let port = 8883;
const facility = await startMixedMqttProxy({
  fixture: '/fixture', broker: '127.0.0.1:1883', report, ownedSockets: sockets,
  hostname: 'mio-mqtt.test', dropPublish: () => false,
  listen: async server => {
    const bound = port; port = 8084;
    servers.push(server);
    await new Promise((resolve, reject) => {
      server.once('error', reject); server.listen(bound, process.env.MIO_SERVICE_IP, resolve);
    });
    return bound;
  },
});
function save() { writeFileSync('/evidence/mqtt-service.json', JSON.stringify(report, null, 2)+'\n'); }
console.log(JSON.stringify({ event: 'services-ready', facility }));
for await (const command of createInterface({ input: process.stdin })) {
  if (command === 'snapshot') { save(); console.log(JSON.stringify({ event: 'services-snapshot', report })); }
  if (command === 'stop') break;
}
for (const socket of sockets) socket.destroy();
await Promise.all(servers.map(server => new Promise(resolve => server.close(resolve))));
save();
