# Hogyoku Raft di Kubernetes

Repo ini menjalankan key-value store Raft sebagai StatefulSet dan UI bawaan sebagai Deployment terpisah. `api_gateway.rs` dan `index.html` tetap utuh. Perubahan aplikasi hanya menyentuh node Raft, resolusi alamat, penyimpanan, dan health endpoint.

## Cakupan yang dikerjakan

| Bagian | Implementasi |
| --- | --- |
| StatefulSet dan headless Service | 3 pod awal bernama `raft-0` sampai `raft-2`, masing-masing punya DNS tetap di `raft-svc` |
| API Gateway | Deployment 2 replika, ClusterIP Service, dan `RAFT_ADDR` dari ConfigMap |
| ConfigMap | ID node, daftar peer, port, lokasi data, ukuran cluster awal, dan contact node berada di `k8s/base/10-configmap.yaml` |
| Persistent storage | Satu PVC `ReadWriteOnce` per pod melalui `volumeClaimTemplates` |
| Crash recovery | term, vote, log, commit index, membership, dan snapshot ditulis ke `/data` |
| Probe | `/ready` baru sukses setelah leader terpilih atau diketahui. `/live` mengecek actor Raft masih merespons |
| Scaling bonus | Ordinal 3 dan 4 otomatis memanggil `AddNode` ke cluster yang sedang hidup |
| NetworkPolicy bonus | Default deny, RPC hanya untuk Raft dan gateway, akses dari luar hanya ke port HTTP gateway |
| Namespace dan RBAC bonus | Semua resource berada di namespace `hogyoku`. ServiceAccount tidak mendapat token atau izin Kubernetes API |

Link:

- Repository: `https://github.com/bellechillguy/Raft-Hogyoku-in-Kubernetes`
- Video demo: `https://youtu.be/aY3Tl-msO1A`

## Kenapa StatefulSet

Setiap anggota Raft punya identitas dan disk yang tidak boleh tertukar. `raft-1` harus kembali sebagai node 2 dengan PVC yang sama setelah restart. Deployment tidak memberi pasangan identitas, hostname, dan volume yang stabil seperti itu.

StatefulSet ini memakai `podManagementPolicy: Parallel`. Jika pod dibuat berurutan, Kubernetes akan menunggu `raft-0` Ready sebelum membuat `raft-1`. Sementara itu, `raft-0` belum bisa Ready karena cluster 3 node belum punya quorum. Mode paralel memutus kebuntuan tersebut tanpa menghilangkan identitas StatefulSet.

Headless Service mengeluarkan record DNS langsung untuk setiap pod. Node tidak perlu menyimpan IP yang mudah berubah:

```text
raft-0.raft-svc.hogyoku.svc.cluster.local:8000
raft-1.raft-svc.hogyoku.svc.cluster.local:8000
raft-2.raft-svc.hogyoku.svc.cluster.local:8000
```

## Struktur Directory

```text
.
├── Dockerfile
├── docker-entrypoint.sh
├── k8s/
│   ├── base/
│   │   ├── 00-namespace-rbac.yaml
│   │   ├── 10-configmap.yaml
│   │   ├── 20-services.yaml
│   │   ├── 30-raft-statefulset.yaml
│   │   ├── 40-api-gateway.yaml
│   │   └── 50-network-policy.yaml
│   └── overlays/https/
├── scripts/cluster-status.sh
├── src/
└── tests/
```

## Build dan deploy lokal

Butuh Docker, `kubectl`, dan cluster Kubernetes aktif. NetworkPolicy hanya benar-benar bisa didemokan jika CNI cluster menegakkannya.

Build image dari folder ini:

```bash
docker build -t hogyoku:local .
```

Docker Desktop Kubernetes dapat memakai image lokal tersebut. Untuk kind atau minikube, muat image ke cluster lebih dulu:

```bash
kind load docker-image hogyoku:local
# atau
minikube image load hogyoku:local
```

Deploy dari nol:

```bash
kubectl delete namespace hogyoku --ignore-not-found
kubectl apply -k k8s
kubectl -n hogyoku get pods -w
```

Pada awal boot, kolom `READY` akan tetap `0/1`. Setelah election selesai, tiga pod Raft berubah menjadi `1/1`. Cek resource dan PVC:

```bash
kubectl -n hogyoku get statefulset,deployment,service,pod,pvc
kubectl -n hogyoku get endpointslice -l kubernetes.io/service-name=raft-svc
```

Lihat status tiap node:

```bash
./scripts/cluster-status.sh
```

Output `/status` berisi role, term, leader ID, commit index, dan jumlah peer. Tepat satu node semestinya memiliki `"role":"leader"`.

## Membuka UI dan menguji data

Jalankan port-forward di terminal terpisah:

```bash
kubectl -n hogyoku port-forward service/api-gateway 8080:8080
```

Buka `http://127.0.0.1:8080`, lalu lakukan Set dan Get dari UI. Tes yang sama bisa dijalankan lewat HTTP:

```bash
curl -sS -X POST http://127.0.0.1:8080/api/set \
  -H 'content-type: application/json' \
  -d '{"key":"fragment","value":"still-here"}'

curl -sS http://127.0.0.1:8080/api/get/fragment
```

## Demo crash recovery

Simpan data dari UI, lalu cari leader dengan `./scripts/cluster-status.sh`. Hapus pod leader saja, bukan PVC:

```bash
kubectl -n hogyoku delete pod raft-LEADER_ORDINAL
kubectl -n hogyoku get pods -w
./scripts/cluster-status.sh
```

StatefulSet membuat pod pengganti dengan nama yang sama dan memasangkan PVC lamanya. Leader baru dipilih oleh dua node yang masih hidup. Setelah pod pengganti Ready, Get nilai `fragment` lagi dari UI.

Untuk waktu recovery, ambil waktu sebelum delete dan berhenti saat status kembali menunjukkan satu leader:

```bash
python3 -c 'import time; print(int(time.time() * 1000))'
kubectl -n hogyoku delete pod raft-LEADER_ORDINAL
while ! ./scripts/cluster-status.sh | grep -q '"role":"leader"'; do sleep 1; done
python3 -c 'import time; print(int(time.time() * 1000))'
```

Selisih kedua angka adalah waktu recovery dalam milidetik. Target tugas untuk pergantian leader adalah kurang dari 10 detik. 


## Scale dari 3 ke 5 node

Tidak perlu mengubah image atau me-restart tiga node lama:

```bash
kubectl -n hogyoku scale statefulset raft --replicas=5
kubectl -n hogyoku rollout status statefulset raft --timeout=180s
kubectl -n hogyoku get pods
./scripts/cluster-status.sh
```

`raft-3` mengambil ID 4 dan `raft-4` mengambil ID 5 dari ConfigMap. Entrypoint melihat ordinal keduanya berada di luar cluster awal, lalu menghubungi `raft-0`. Jika `raft-0` bukan leader, RPC mengembalikan alamat leader dan proses join mengikuti redirect. Leader menulis `AddNode` ke log dan mengirim log atau snapshot kepada node baru.

Ulangi Get untuk data yang dibuat sebelum scaling. Nilainya harus tetap ada.

## Demo NetworkPolicy

Pastikan CNI cluster mendukung NetworkPolicy. Port gateway boleh diakses, sedangkan RPC Raft dari namespace lain harus gagal:

```bash
kubectl run outside-check \
  --image=busybox:1.36 \
  --restart=Never \
  --rm -it \
  -- nc -zvw3 raft-0.raft-svc.hogyoku.svc.cluster.local 8000
```

Perintah tersebut semestinya timeout. UI tetap berfungsi karena pod gateway diberi akses khusus ke port 8000. Tampilkan policy saat menjelaskan hasilnya:

```bash
kubectl -n hogyoku get networkpolicy
kubectl -n hogyoku describe networkpolicy raft-internal
```

## Demo RBAC

Workload ini tidak perlu membaca Pod, Secret, atau resource Kubernetes lain. Karena itu Role sengaja memiliki `rules: []`, token ServiceAccount tidak di-mount, dan kedua ServiceAccount diikat ke Role kosong tersebut.

```bash
kubectl auth can-i \
  --as=system:serviceaccount:hogyoku:raft \
  get pods -n hogyoku

kubectl auth can-i \
  --as=system:serviceaccount:hogyoku:api-gateway \
  get secrets -n hogyoku
```

Keduanya harus menjawab `no`. Ini lebih kecil izinnya daripada memberi akses yang tidak digunakan.


## Verifikasi source

```bash
rustfmt --edition 2021 --check src/raft/state.rs src/raft/actor.rs src/bin/server.rs src/utils/client.rs tests/persistent_recovery.rs
cargo test --locked --all-targets
kubectl kustomize k8s >/tmp/hogyoku-rendered.yaml
kubectl kustomize k8s/overlays/https >/tmp/hogyoku-https-rendered.yaml
```

Test mencakup operasi KV, log conflict, snapshot, membership command, dan recovery data committed sebelum snapshot dibuat.
