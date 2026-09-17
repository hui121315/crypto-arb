use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialPermission {
    OpenOrdersRead,
    PlaceOrder,
    CancelOrder,
}

impl VenueCredentialPermission {
    pub const ALL: [Self; 3] = [Self::OpenOrdersRead, Self::PlaceOrder, Self::CancelOrder];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenOrdersRead => "open_orders_read",
            Self::PlaceOrder => "place_order",
            Self::CancelOrder => "cancel_order",
        }
    }

    pub const fn probe_kind(self) -> &'static str {
        match self {
            Self::OpenOrdersRead => "open_orders_read",
            Self::PlaceOrder | Self::CancelOrder => "order_permission",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialPermissionStatus {
    Validated,
    Denied,
    Unproven,
    Missing,
}

impl VenueCredentialPermissionStatus {
    pub const fn is_validated(self) -> bool {
        matches!(self, Self::Validated)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validated => "validated",
            Self::Denied => "denied",
            Self::Unproven => "unproven",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialPermissionEvidence {
    pub permission: VenueCredentialPermission,
    pub status: VenueCredentialPermissionStatus,
    pub probe_kind: String,
    pub permission_scope: String,
    pub source: String,
    pub message: String,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl VenueCredentialPermissionEvidence {
    fn from_probe(
        permission: VenueCredentialPermission,
        checked_at_ms: i64,
        probe: Option<&VenueCredentialProbe>,
    ) -> Self {
        let Some(probe) = probe else {
            return Self::missing(permission, checked_at_ms);
        };
        let status = match probe.status {
            VenueCredentialProbeStatus::Ok => VenueCredentialPermissionStatus::Validated,
            VenueCredentialProbeStatus::Failed => VenueCredentialPermissionStatus::Denied,
            VenueCredentialProbeStatus::Unknown => VenueCredentialPermissionStatus::Unproven,
        };
        Self {
            permission,
            status,
            probe_kind: probe.kind.clone(),
            permission_scope: permission.as_str().to_owned(),
            source: probe.source.clone(),
            message: probe.message.clone(),
            checked_at_ms: probe.checked_at_ms,
            request_id: probe.request_id.clone(),
            error: (probe.status == VenueCredentialProbeStatus::Failed)
                .then(|| probe.message.clone()),
        }
    }

    fn missing(permission: VenueCredentialPermission, checked_at_ms: i64) -> Self {
        Self {
            permission,
            status: VenueCredentialPermissionStatus::Missing,
            probe_kind: permission.probe_kind().to_owned(),
            permission_scope: permission.as_str().to_owned(),
            source: "not_probed".to_owned(),
            message: format!("{} permission evidence is missing", permission.as_str()),
            checked_at_ms,
            request_id: None,
            error: None,
        }
    }
}

impl VenueCredentialValidationEvidence {
    pub fn with_order_permission_scopes(mut self, scopes: &[VenueCredentialPermission]) -> Self {
        let open_orders = self
            .probes
            .iter()
            .find(|probe| probe.kind == VenueCredentialPermission::OpenOrdersRead.probe_kind());
        let order_permission = self
            .probes
            .iter()
            .find(|probe| probe.kind == VenueCredentialPermission::PlaceOrder.probe_kind());
        self.permission_evidence = VenueCredentialPermission::ALL
            .into_iter()
            .map(|permission| {
                let probe = match permission {
                    VenueCredentialPermission::OpenOrdersRead => open_orders,
                    VenueCredentialPermission::PlaceOrder
                    | VenueCredentialPermission::CancelOrder
                        if scopes.contains(&permission) =>
                    {
                        order_permission
                    }
                    VenueCredentialPermission::PlaceOrder
                    | VenueCredentialPermission::CancelOrder => None,
                };
                VenueCredentialPermissionEvidence::from_probe(permission, self.checked_at_ms, probe)
            })
            .collect();
        self
    }

    pub fn permission(
        &self,
        permission: VenueCredentialPermission,
    ) -> Option<&VenueCredentialPermissionEvidence> {
        self.permission_evidence
            .iter()
            .find(|evidence| evidence.permission == permission)
    }
}
