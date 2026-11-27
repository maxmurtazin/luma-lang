        #[cfg(feature = "luma-embedded-2d")]
        ("2d-propose-branch", [part, project_revision, batch, destination]) => {
            let expected = project_revision.parse().map_err(|_| usage())?;
            let (preview, token) = store.propose_2d_branch(part, expected, &read(batch, 256 * 1024)?, &context)?;
            write_token(destination, &token)?;
            Ok(preview)
        }
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-accept-branch", [token, operation]) => {
            validate_operation(operation)?;
            let receipt = store.accept_2d_branch(&read(token, 2 * 1024 * 1024)?, operation, &context)?;
            let mut result = receipt_json(&receipt);
            result["embedded_revision"] = json!(store.embedded_2d_revision_at(&receipt)?.to_string());
            Ok(result)
        }
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-register" | "2d-apply", [part, project_revision, operation, file]) => {
            let expected = project_revision.parse().map_err(|_| usage())?;
            validate_operation(operation)?;
            let receipt = if action == "2d-register" {
                store.register_embedded_2d(part, operation, expected, &read(file, 4 * 1024 * 1024)?, &context)?
            } else {
                store.apply_embedded_2d(part, operation, expected, &read(file, 256 * 1024)?, None, &context)?
            };
            let mut result = receipt_json(&receipt);
            result["embedded_revision"] = json!(store.embedded_2d_revision_at(&receipt)?.to_string());
            Ok(result)
        }
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-length-source", [document, entity]) => store.embedded_2d_length_source(document, entity, &context),
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-inspect", [part]) => store.embedded_2d(part, None),
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-solve-report", [part, sketch]) => store.embedded_2d(part, Some(sketch)),
        #[cfg(feature = "luma-embedded-2d")]
        ("2d-luma", [part, project_revision, embedded_revision, operation, file]) => {
            let expected = project_revision.parse().map_err(|_| usage())?;
            let embedded: u64 = embedded_revision.parse().map_err(|_| usage())?;
            validate_operation(operation)?;
            let bundle: luma::host::WireSourceBundle = serde_json::from_slice(&read(file, 256 * 1024)?)
                .map_err(|_| error("InvalidSourceBundle", "Expected a Luma source bundle"))?;
            let bundle = bundle.admit().map_err(|problem| {
                error("InvalidSourceBundle", &format!("{:?}", problem.diagnostics))
            })?;
            let plan = luma::cad::prepare_sketch(&bundle, 100_000).map_err(|problem| {
                error("Embedded2dPlanRejected", &format!("{problem:?}"))
            })?;
            let request = format!("{}-{}-{}-{}-{}",
                &operation[..8], &operation[8..12], &operation[12..16], &operation[16..20], &operation[20..]);
            let batch = json!({
                "api_version":"1.0", "request_id":request, "project_id":project, "document_id":part,
                "expected_revision":embedded.to_string(),
                "idempotency_key":format!("luma-{}",bundle.source_digest().trim_start_matches("sha256:")),
                "commands":plan.commands, "execution_policy":{
                    "numeric_profile_id":"mechanical-si-v1","source_policy":"pin_current",
                    "solver_policy":"preserve_branch","timeout_ms":5000,"max_memory_bytes":50331648
                }
            });
            let bytes = serde_json::to_vec(&batch).map_err(living_cad_viewer::json_error)?;
            let receipt = store.apply_embedded_2d(part, operation, expected, &bytes, plan.extrusion.as_ref(), &context)?;
            let mut result = receipt_json(&receipt);
            result["embedded_revision"] = json!(store.embedded_2d_revision_at(&receipt)?.to_string());
            Ok(result)
        }
