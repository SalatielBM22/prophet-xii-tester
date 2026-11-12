// SPDX-License-Identifier: GPL-3.0-only
// Standalone ABI/state regression. Compile against official CLAP 1.2.6 headers.
#include <clap/clap.h>
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
static const void *extension(const clap_host_t *h,const char *id){return NULL;}
static void noop(const clap_host_t *h){}
static clap_event_param_value_t event;
static uint32_t count(const clap_input_events_t *l){return 1;}
static const clap_event_header_t *get(const clap_input_events_t *l,uint32_t i){return &event.header;}
static bool push(const clap_output_events_t *l,const clap_event_header_t *e){return true;}
struct Memory{unsigned char bytes[65536];size_t len,pos;};
static int64_t write_state(const clap_ostream_t *s,const void *data,uint64_t n){struct Memory *m=s->ctx;if(m->len+n>sizeof m->bytes)return -1;memcpy(m->bytes+m->len,data,n);m->len+=n;return n;}
static int64_t read_state(const clap_istream_t *s,void *data,uint64_t n){struct Memory *m=s->ctx;if(n>m->len-m->pos)n=m->len-m->pos;memcpy(data,m->bytes+m->pos,n);m->pos+=n;return n;}
static void set(const clap_plugin_t *p,const clap_plugin_params_t *params,clap_id id,double value){event=(clap_event_param_value_t){.header={.size=sizeof(event),.time=0,.space_id=CLAP_CORE_EVENT_SPACE_ID,.type=CLAP_EVENT_PARAM_VALUE},.param_id=id,.cookie=NULL,.note_id=-1,.port_index=-1,.channel=-1,.key=-1,.value=value};clap_input_events_t input={.size=count,.get=get};clap_output_events_t output={.try_push=push};params->flush(p,&input,&output);}
int main(int argc,char **argv){assert(argc==2);void *lib=dlopen(argv[1],RTLD_NOW);assert(lib);const clap_plugin_entry_t *entry=dlsym(lib,"clap_entry");assert(entry&&entry->init(argv[1]));const clap_plugin_factory_t *factory=entry->get_factory(CLAP_PLUGIN_FACTORY_ID);assert(factory);const clap_plugin_descriptor_t *desc=factory->get_plugin_descriptor(factory,0);clap_host_t host={.clap_version=CLAP_VERSION_INIT,.name="Agent B regression",.vendor="Local",.url="https://localhost",.version="1",.get_extension=extension,.request_restart=noop,.request_process=noop,.request_callback=noop};const clap_plugin_t *p=factory->create_plugin(factory,&host,desc->id);assert(p&&p->init(p));const clap_plugin_params_t *params=p->get_extension(p,CLAP_EXT_PARAMS);assert(params&&params->count(p)==85);
 clap_id enabled=CLAP_INVALID_ID,kill=CLAP_INVALID_ID;for(uint32_t i=0;i<85;i++){clap_param_info_t info;assert(params->get_info(p,i,&info));if(!strcmp(info.name,"Reverb Enabled"))enabled=info.id;if(!strcmp(info.name,"Reverb Kill Tail (off = Spill)"))kill=info.id;}assert(enabled!=CLAP_INVALID_ID&&kill!=CLAP_INVALID_ID&&enabled!=kill);double v;assert(params->get_value(p,enabled,&v)&&v==1);assert(params->get_value(p,kill,&v)&&v==0);
 set(p,params,enabled,0);set(p,params,kill,1);assert(params->get_value(p,enabled,&v)&&v==0);assert(params->get_value(p,kill,&v)&&v==1);
 const clap_plugin_state_t *state=p->get_extension(p,CLAP_EXT_STATE);assert(state);struct Memory mem={0};clap_ostream_t out={.ctx=&mem,.write=write_state};assert(state->save(p,&out));set(p,params,enabled,1);set(p,params,kill,0);clap_istream_t in={.ctx=&mem,.read=read_state};assert(state->load(p,&in));assert(params->get_value(p,enabled,&v)&&v==0);assert(params->get_value(p,kill,&v)&&v==1);printf("PASS: 83 legacy IDs plus two reverb controls; enable/kill default, edit, state save/restore (%zu bytes)\n",mem.len);p->destroy(p);entry->deinit();dlclose(lib);return 0;}
