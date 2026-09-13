terraform {
  required_version = ">= 1.11"
  required_providers {
    hcloud = {
      source = "hetznercloud/hcloud", version = "~> 1.62.0"
    }

  }
  # Dedicated local state: never initialize in or reuse cc-remote's state directory.
  encryption {
    key_provider "pbkdf2" "state" {
      passphrase = var.state_passphrase
    }
    method "aes_gcm" "state" {
      keys = key_provider.pbkdf2.state
    }
    state {
      method   = method.aes_gcm.state
      enforced = true
    }
    plan {
      method   = method.aes_gcm.state
      enforced = true
    }

  }
}
variable "hcloud_token" {
  type      = string
  sensitive = true
}
variable "state_passphrase" {
  type      = string
  sensitive = true
}
variable "ssh_public_key" {
  type = string
}
variable "admin_ipv4_cidr" {
  type = string
  validation {
    condition     = can(cidrhost(var.admin_ipv4_cidr, 0)) && endswith(var.admin_ipv4_cidr, "/32")
    error_message = "Use the operator's single IPv4 address with /32."
  }
}
variable "enable_public_https" {
  type    = bool
  default = false
}
provider "hcloud" {
  token = var.hcloud_token
}
resource "hcloud_ssh_key" "pds" {
  name       = "linkjar-pds"
  public_key = var.ssh_public_key
}
resource "hcloud_firewall" "pds" {
  name = "linkjar-pds"
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = [var.admin_ipv4_cidr]
  }
  rule {
    direction  = "in"
    protocol   = "icmp"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  dynamic "rule" {
    for_each = var.enable_public_https ? [1] : []
    content {
      direction  = "in"
      protocol   = "tcp"
      port       = "443"
      source_ips = ["0.0.0.0/0", "::/0"]
    }

  }
}
resource "hcloud_server" "pds" {
  name         = "linkjar-pds"
  server_type  = "cax21"
  location     = "nbg1"
  image        = "ubuntu-24.04"
  ssh_keys     = [hcloud_ssh_key.pds.id]
  firewall_ids = [hcloud_firewall.pds.id]
  public_net {
    ipv4_enabled = true
    ipv6_enabled = false
  }
  lifecycle {
    prevent_destroy = true
    ignore_changes  = [image, ssh_keys]
  }
}
output "server_id" {
  value = hcloud_server.pds.id
}
output "server_ipv4" {
  value = hcloud_server.pds.ipv4_address
}
