#include <arpa/inet.h>
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static double ms(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return t.tv_sec*1000.0+t.tv_nsec/1e6; }
static int probe(int fd,const char *label,const char *ip,int port) {
  struct sockaddr_in target={.sin_family=AF_INET,.sin_port=htons(port)},source;
  if(inet_pton(AF_INET,ip,&target.sin_addr)!=1) return 2;
  unsigned char request[20]={0,1,0,0,0x21,0x12,0xa4,0x42},reply[2048];
  int randomfd=open("/dev/urandom",O_RDONLY);if(randomfd<0)return 2;
  ssize_t randomsize=read(randomfd,request+8,12);close(randomfd);if(randomsize!=12)return 2;
  double start=ms();int match=0,received=0,source_matches=0; char address[INET_ADDRSTRLEN]="";int sourceport=0;
  for(int retry=0;retry<2&&!match;retry++) {
    if(sendto(fd,request,sizeof(request),0,(struct sockaddr*)&target,sizeof(target))!=20)return 2;
    double end=ms()+1200;
    while(ms()<end&&!match) {
      struct pollfd p={.fd=fd,.events=POLLIN}; if(poll(&p,1,(int)(end-ms()))<=0)break;
      socklen_t slen=sizeof(source);ssize_t n=recvfrom(fd,reply,sizeof(reply),0,(struct sockaddr*)&source,&slen);
      if(n<20)continue;received++;
      if(!memcmp(reply+4,request+4,16)) {
        match=1;source_matches=source.sin_addr.s_addr==target.sin_addr.s_addr&&source.sin_port==target.sin_port;
        inet_ntop(AF_INET,&source.sin_addr,address,sizeof(address));sourceport=ntohs(source.sin_port);
      }
    }
  }
  printf("{\"label\":\"%s\",\"matchedTransaction\":%s,\"sourceMatchesRequestedDestination\":%s,\"source\":\"%s\",\"sourcePort\":%d,\"replyCount\":%d,\"wallMs\":%.3f}\n",label,match?"true":"false",source_matches?"true":"false",address,sourceport,received,ms()-start);
  fflush(stdout);return 0;
}
int main(int argc,char**argv) {
  if(argc!=5)return 2;int fd=socket(AF_INET,SOCK_DGRAM,0);if(fd<0)return 2;
  if(probe(fd,"first-A",argv[1],atoi(argv[2]))||probe(fd,"same-socket-B",argv[3],atoi(argv[4])))return 2;
  close(fd);fd=socket(AF_INET,SOCK_DGRAM,0);if(fd<0)return 2;
  int rc=probe(fd,"fresh-socket-B",argv[3],atoi(argv[4]));close(fd);return rc;
}
