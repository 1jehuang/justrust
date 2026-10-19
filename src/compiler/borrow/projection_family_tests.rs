use super::*;

#[cfg(test)]
mod projection_family_transport_tests {
    use super::*;

    fn projection() -> Arc<ir::AssociatedTypeProjection> {
        let snapshot = ir::SnapshotId::allocate().unwrap();
        Arc::new(ir::AssociatedTypeProjection {
            trait_ref: ir::TraitRef {
                declaration: ir::DeclId { snapshot, index: 1 },
                self_ty: Ty::Int(ir::IntTy::I32),
                args: Arc::from([Ty::Int(ir::IntTy::U8), Ty::Int(ir::IntTy::U16)]),
            },
            item: ir::DeclId { snapshot, index: 2 },
        })
    }

    #[test]
    fn projection_carrier_families_preserve_exact_structural_identity() {
        let original = projection();
        let family = ir::CarrierFamily::Projection(original.clone());
        let channel = family_channel(&family, 2);
        assert_eq!(channel.family(), Some(family.clone()));
        assert_eq!(channel.prefix(), 2);
        assert_eq!(
            family_region_term(&family),
            ir::CarrierRegionTerm::ProjectionFamily {
                projection: original.clone()
            }
        );
        let location = CarrierLocation {
            path: CarrierPath(Arc::from([])),
            channel: family_channel(&family, 0),
        };
        let budget = ir::PlanBudget::default();
        assert!(ir::carrier_at(&Ty::AssociatedType(original.clone()), &location, &budget).is_ok());
        for mutation in 0..5 {
            let mut changed = original.as_ref().clone();
            match mutation {
                0 => changed.trait_ref.declaration.index += 1,
                1 => changed.item.index += 1,
                2 => changed.trait_ref.self_ty = Ty::Int(ir::IntTy::I64),
                3 => {
                    changed.trait_ref.args =
                        Arc::from([Ty::Int(ir::IntTy::U16), Ty::Int(ir::IntTy::U8)])
                }
                _ => changed.item.snapshot = ir::SnapshotId::allocate().unwrap(),
            }
            let changed = Arc::new(changed);
            let other = ir::CarrierFamily::Projection(changed.clone());
            assert_ne!(family, other);
            assert_ne!(family_region_term(&family), family_region_term(&other));
            assert!(
                ir::carrier_at(&Ty::AssociatedType(changed), &location, &budget).is_err(),
                "foreign projection mutation {mutation}"
            );
        }
    }

    #[test]
    fn open_projection_is_not_a_zero_carrier_scalar() {
        let budget = ir::PlanBudget::default();
        let projection = projection();
        let open = Ty::AssociatedType(projection.clone());
        let locations = ir::carrier_locations(&open, &budget).unwrap();
        assert_eq!(locations.len(), 1);
        assert_eq!(
            locations[0].channel.family(),
            Some(ir::CarrierFamily::Projection(projection))
        );
        assert!(ir::carrier_at(&open, &locations[0], &budget).is_ok());
        assert!(
            ir::carrier_locations(&Ty::Int(ir::IntTy::I32), &budget)
                .unwrap()
                .is_empty()
        );
        assert!(ir::carrier_at(&Ty::Int(ir::IntTy::I32), &locations[0], &budget).is_err());
    }
}
