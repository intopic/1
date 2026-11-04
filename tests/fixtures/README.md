# Fixture testuese – VETËM PËR TESTE

* `mock_notary_key.pem` / `mock_notary_cert.pem` – çelës RSA-2048 dhe certifikatë e vetë-nënshkruar
  e një "noteri" fiktiv (`CN=mock-notary.zktls.test`). Çelësi privat është publik me qëllim;
  mos e përdorni kurrë jashtë testeve.
* `dev_proof_fixture.json` – VK + provë Groth16 e qarkut DEV, e gjeneruar me
  `ZKTLS_WRITE_FIXTURES=1 cargo test -p zktls-prover --test e2e_mock_tls`; përdoret nga testet
  native të programit Anchor.
