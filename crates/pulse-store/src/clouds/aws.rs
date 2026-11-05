//! Amazon Web Services: EC2 instances through the aws CLI's profile.

use super::{Cloud, field, mode};

pub fn descriptor() -> Cloud {
    Cloud {
        id: "aws".into(),
        label: "Amazon Web Services".into(),
        blurb: "EC2 instances made and removed with the AWS CLI (aws) on this machine.".into(),
        auth_setting: String::new(),
        auth: vec![mode(
            "login",
            "An AWS CLI profile",
            "aws configure or aws sso login on this machine; PULSE uses the profile named below, or the default one.",
            "aws sso login",
            vec![],
        )],
        fields: vec![
            field(
                "compute.aws.profile",
                "Profile",
                "The AWS CLI profile. Empty: the default one.",
                false,
                "default",
            ),
            field(
                "compute.aws.region",
                "Region",
                "The region the instances run in. Empty: the profile's default region.",
                false,
                "eu-west-1",
            ),
            field(
                "compute.aws.instance_type",
                "Instance type",
                "The EC2 instance type each machine gets.",
                false,
                "t3.medium",
            ),
            field(
                "compute.aws.image",
                "Image",
                "An AMI id or an SSM public parameter path.",
                false,
                "",
            ),
            field(
                "compute.aws.subnet",
                "Subnet",
                "The subnet id. Empty: the default VPC's default subnet.",
                false,
                "",
            ),
            field(
                "compute.aws.security_group",
                "Security group",
                "The security group id. Empty: one named pulse-hands, made when missing.",
                false,
                "",
            ),
        ],
        backup_target: String::new(),
    }
}
