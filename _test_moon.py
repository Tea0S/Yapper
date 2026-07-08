import json, subprocess, sys, time
p = subprocess.Popen([sys.executable, r"C:\Users\Bambi\AppData\Local\Yapper\_up_\sidecar\server.py"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
msgs = [
  {"type":"init","model":"base","device":"cpu","compute_type":"int8","model_dir":"C:/Users/Bambi/AppData/Local/com.yapper.app/models","mock":False,"engine":"whisper","lazy_load":True},
  {"type":"start_stream","session_id":42,"engine":"moonshine","model":"small_streaming"},
]
for m in msgs:
    p.stdin.write(json.dumps(m)+"\n"); p.stdin.flush()
time.sleep(2)
p.stdin.close()
out, err = p.communicate(timeout=5)
print("OUT:", out)
if "stream_started" in out:
    print("OK stream_started")
else:
    print("ERR tail:", err[-800:])
