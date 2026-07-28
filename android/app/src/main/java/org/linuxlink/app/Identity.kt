package org.linuxlink.app

import android.content.Context
import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.X509v3CertificateBuilder
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import java.io.File
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.PKCS8EncodedKeySpec
import java.util.Date

class Identity private constructor(
    val certificate: X509Certificate,
    val privateKey: PrivateKey,
) {
    val fingerprint: String
        get() = MessageDigest.getInstance("SHA-256")
            .digest(certificate.encoded)
            .joinToString("") { "%02x".format(it) }

    companion object {
        private const val CERT_FILE = "identity-cert.der"
        private const val KEY_FILE = "identity-key.pk8"

        fun loadOrCreate(context: Context): Identity {
            val certFile = File(context.filesDir, CERT_FILE)
            val keyFile = File(context.filesDir, KEY_FILE)

            if (certFile.exists() && keyFile.exists()) {
                val cert = CertificateFactory.getInstance("X.509")
                    .generateCertificate(certFile.inputStream()) as X509Certificate
                val key = KeyFactory.getInstance("EC")
                    .generatePrivate(PKCS8EncodedKeySpec(keyFile.readBytes()))
                return Identity(cert, key)
            }

            val keyPair = KeyPairGenerator.getInstance("EC").apply { initialize(256) }.genKeyPair()
            val cert = selfSign(keyPair, "linuxlink-android")
            certFile.writeBytes(cert.encoded)
            keyFile.writeBytes(keyPair.private.encoded)
            return Identity(cert, keyPair.private)
        }

        private fun selfSign(keyPair: KeyPair, cn: String): X509Certificate {
            val now = Date()
            val until = Date(now.time + 20L * 365 * 24 * 3600 * 1000)
            val name = X500Name("CN=$cn")
            val builder: X509v3CertificateBuilder = JcaX509v3CertificateBuilder(
                name, BigInteger.valueOf(now.time), now, until, name, keyPair.public
            )
            val signer = JcaContentSignerBuilder("SHA256withECDSA").build(keyPair.private)
            return JcaX509CertificateConverter().getCertificate(builder.build(signer))
        }
    }
}

data class PairedPc(
    val name: String,
    val lastAddress: String,
    val port: Int,
    val fingerprint: String,
) {
    companion object {
        private const val PREFS = "paired_pc"

        fun load(context: Context): PairedPc? {
            val p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val name = p.getString("name", null) ?: return null
            return PairedPc(
                name = name,
                lastAddress = p.getString("address", "")!!,
                port = p.getInt("port", 47100),
                fingerprint = p.getString("fingerprint", "")!!,
            )
        }

        fun save(context: Context, pc: PairedPc) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putString("name", pc.name)
                .putString("address", pc.lastAddress)
                .putInt("port", pc.port)
                .putString("fingerprint", pc.fingerprint)
                .apply()
        }
    }
}
