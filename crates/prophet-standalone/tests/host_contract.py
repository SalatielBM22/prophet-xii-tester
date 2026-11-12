"""Native null-host contract test. stdlib + Node >=22; no React/analysis edits."""
import json, pathlib, subprocess, tempfile, time, urllib.request, urllib.error
ROOT = pathlib.Path(__file__).resolve().parents[3]
BASE = 'http://127.0.0.1:18878'
def get(path):
    return json.load(urllib.request.urlopen(BASE + path, timeout=5))
def send(body):
    return json.load(urllib.request.urlopen(urllib.request.Request(BASE + '/api/v1/command',data=json.dumps(body).encode(),headers={'Content-Type':'application/json'}),timeout=5))
def until(predicate):
    limit=time.monotonic()+5
    while time.monotonic()<limit:
        if predicate():return
        time.sleep(.01)
    raise AssertionError('state acknowledgment timeout')
with tempfile.TemporaryFile() as log:
    process=subprocess.Popen([str(ROOT/'target/release/prophet-standalone'),'--null','--bind','127.0.0.1:18878','--buffer-frames','128','--sample-rate','96000','--duration','30'],cwd=ROOT,stdout=log,stderr=log)
    try:
        for _ in range(100):
            try:
                if get('/api/v1/state').get('ready'):break
            except (OSError,urllib.error.URLError):pass
            time.sleep(.03)
        else:raise AssertionError('host did not become ready')
        s=get('/api/v1/state');assert len(s['values'])==83 and len(s['ampEnvelopes'])==12
        assert s['callback']['frames']==128 and s['sampleRate']==96000
        assert s['callback']['backendBufferedUs'] is None and s['driverXruns'] is None
        meta=get('/api/v1/meta');assert len(meta['presets'])==10
        assert all(len(p['values'])==83 for p in meta['presets'])
        for bad in [{'type':'remap','cc':64,'id':23},{'type':'param','id':23,'value':-1}]:
            try:send(bad);raise AssertionError('invalid command accepted')
            except urllib.error.HTTPError as e:assert e.code==400
        r=send({'type':'reverb_bypass','enabled':False,'tail':'kill'})
        until(lambda:get('/api/v1/state')['control']['lastAppliedCommandId']==r['acceptedCommandId'])
        s=get('/api/v1/state');assert not s['reverb']['enabled'] and s['reverb']['dormant']
        send({'type':'learn','id':23});send({'type':'cancel_learn'})
        send({'type':'remap','cc':16,'id':23});until(lambda:get('/api/v1/state')['midiLearn']['ccMap'][16]==23)
        send({'type':'unlearn','id':23});until(lambda:get('/api/v1/state')['midiLearn']['ccMap'][16] is None)
        send({'type':'midi','status':144,'data1':60,'data2':100})
        until(lambda:get('/api/v1/state')['activeVoiceMask']!=0)
        s=get('/api/v1/state');assert any(e['stage'] in [1,2,3] for e in s['ampEnvelopes'])
        assert s['revision']>0 and s['control']['waitUs']>=s['control']['relayWaitUs']
        js="""
const ws=new WebSocket('ws://127.0.0.1:18878/api/v1/capture');
const timer=setTimeout(()=>{console.error('capture timeout');process.exit(1)},5000);
let blocks=[];let messages=0;
ws.onmessage=e=>{blocks.push(...JSON.parse(e.data).blocks);if(++messages===3){
 for(let i=1;i<blocks.length;i++){const a=blocks[i-1],b=blocks[i];if(b.sequence!==a.sequence+1||b.firstFrame!==a.firstFrame+a.samples.length)throw Error('capture gap');}
 if(!blocks.some(b=>b.samples.some(x=>Math.abs(x)>0.00001)))throw Error('capture is not real engine audio');
 console.log(JSON.stringify({blocks:blocks.length,sampleRate:blocks[0].sampleRate,firstFrame:blocks[0].firstFrame}));
 clearTimeout(timer);ws.close();}};
ws.onerror=e=>{if(messages>=3)return;console.error('capture socket error:',e.message,e.error?.message);process.exit(1)};
"""
        result=subprocess.run(['node','--input-type=module','-e',js],capture_output=True,text=True,timeout=10)
        assert result.returncode==0, result.stderr
        capture=json.loads(result.stdout)
        assert capture['sampleRate']==96000
        print(json.dumps({'status':'PROVEN','provenance':'HOST_VERIFIED','checks':['96k/128-frame null host','legacy 83 values','factory snapshots','range validation','bypass acknowledgment','MIDI learn lifecycle','real ADSR stages','sequence-contiguous nonzero PCM via WebSocket'],'capture':capture}))
    finally:
        process.terminate()
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:process.kill();process.wait()
